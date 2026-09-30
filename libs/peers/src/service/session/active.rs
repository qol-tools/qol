use std::{collections::VecDeque, future::Future, time::Duration};

use tokio::{
    io::{split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::{interval_at, timeout_at, Instant, MissedTickBehavior},
};

use crate::{
    service::{
        framing::{read_idle_json, write_json, FrameLimit},
        PeerAuthority, PeerPin,
    },
    session::SessionGeneration,
};

use super::{
    authority_error, frame_error, watch_authority,
    wire::{Message, Version},
    NormalSession, SessionError, SessionOutcome,
};

const HEARTBEAT_PERIOD: Duration = Duration::from_secs(15);
const IDLE_LIMIT: Duration = Duration::from_secs(45);
const RATE_WINDOW: Duration = Duration::from_secs(60);
const RATE_LIMIT: usize = 64;
const UNLINK_DEADLINE: Duration = Duration::from_secs(1);

pub(super) async fn run<S: AsyncRead + AsyncWrite + Unpin>(
    session: NormalSession<S>,
    cancellation: impl Future<Output = ()>,
    operations: Option<(
        std::sync::Arc<crate::service::network::operations::Hub>,
        crate::service::network::operations::SessionIo,
    )>,
) -> SessionOutcome {
    let NormalSession {
        connection,
        authority,
        authenticated,
        opened,
    } = session;
    let (hub, io) = match operations {
        Some((hub, io)) => (Some(hub), Some(io)),
        None => (None, None),
    };
    let pin = connection.remote_identity().pin().clone();
    let changes = authority.watch_changes();
    let (mut reader, mut writer) = split(connection);
    let outcome = tokio::select! {
        biased;
        () = cancellation => SessionOutcome::Cancelled,
        error = watch_authority(&authority, &pin, changes) => SessionOutcome::Closed(error),
        error = receive(&mut reader, &authority, &pin, authenticated, opened, hub.as_deref()) => SessionOutcome::Closed(error),
        error = send(&mut writer, &authority, &pin, authenticated.generation, opened, io) => SessionOutcome::Closed(error),
    };
    if matches!(
        authority.normal_session_identity(&pin),
        Err(crate::AuthorityError::UnknownPeer)
    ) {
        let unlinked = Message::Unlinked {
            version: Version,
            sender_nonce: authenticated.generation.local,
            recipient_nonce: authenticated.generation.remote,
        };
        let _ = tokio::time::timeout(UNLINK_DEADLINE, async {
            if write_json(&mut writer, &unlinked, FrameLimit::Normal)
                .await
                .is_ok()
                && writer.shutdown().await.is_ok()
            {
                await_peer_close_so_no_reset_discards_the_frame(&mut reader).await;
            }
        })
        .await;
    }
    outcome
}

async fn await_peer_close_so_no_reset_discards_the_frame<R: AsyncRead + Unpin>(reader: &mut R) {
    let mut unread = [0; 1024];
    while matches!(reader.read(&mut unread).await, Ok(1..)) {}
}

async fn receive<R: AsyncRead + Unpin>(
    reader: &mut R,
    authority: &PeerAuthority,
    pin: &PeerPin,
    session: crate::session::AuthenticatedSession,
    opened: Instant,
    hub: Option<&crate::service::network::operations::Hub>,
) -> SessionError {
    let generation = session.generation;
    let mut operation_rate = OperationRate::default();
    let mut last_heartbeat = opened;
    let mut rate = ControlRate::default();
    loop {
        let frame = timeout_at(
            last_heartbeat + IDLE_LIMIT,
            read_idle_json::<_, Message>(reader, FrameLimit::Normal),
        )
        .await;
        let message = match frame {
            Err(_) => return SessionError::IdleExpired,
            Ok(Err(error)) => return frame_error(error),
            Ok(Ok(message)) => message,
        };
        if let Err(error) = authority.normal_session_identity(pin) {
            return authority_error(error);
        }
        let now = Instant::now();
        if matches!(
            message,
            Message::Hello { .. } | Message::Heartbeat { .. } | Message::Unlinked { .. }
        ) && !rate.admit(now)
        {
            return SessionError::RateExceeded;
        }
        if matches!(
            message,
            Message::OperationEpoch { .. } | Message::Operation { .. }
        ) && !operation_rate.admit(now)
        {
            return SessionError::RateExceeded;
        }
        match message {
            Message::Heartbeat {
                sender_nonce,
                recipient_nonce,
                ..
            } => {
                if sender_nonce != generation.remote || recipient_nonce != generation.local {
                    return SessionError::Generation;
                }
                last_heartbeat = now;
            }
            Message::OperationEpoch {
                sender_nonce,
                recipient_nonce,
                epoch,
                ..
            } => {
                if sender_nonce != generation.remote || recipient_nonce != generation.local {
                    return SessionError::Generation;
                }
                let Some(hub) = hub else {
                    return SessionError::Protocol;
                };
                if hub.epoch(session, epoch).is_err() {
                    return SessionError::Protocol;
                }
            }
            Message::Operation {
                sender_nonce,
                recipient_nonce,
                message,
                ..
            } => {
                if sender_nonce != generation.remote || recipient_nonce != generation.local {
                    return SessionError::Generation;
                }
                let Some(hub) = hub else {
                    return SessionError::Protocol;
                };
                if hub.receive(session, message).is_err() {
                    return SessionError::Protocol;
                }
            }
            Message::Unlinked {
                sender_nonce,
                recipient_nonce,
                ..
            } => {
                if sender_nonce != generation.remote || recipient_nonce != generation.local {
                    return SessionError::Generation;
                }
                if let Err(error) = authority.unlinked_by(pin.peer_id()) {
                    return authority_error(error);
                }
                return SessionError::Untrusted;
            }
            Message::Hello { .. } => return SessionError::Protocol,
        }
    }
}

async fn send<W: AsyncWrite + Unpin>(
    writer: &mut W,
    authority: &PeerAuthority,
    pin: &PeerPin,
    generation: SessionGeneration,
    opened: Instant,
    mut io: Option<crate::service::network::operations::SessionIo>,
) -> SessionError {
    if io.is_some() {
        let Ok(epoch) = authority.operation_epoch(pin.peer_id()) else {
            return SessionError::AuthorityUnavailable;
        };
        let message = Message::OperationEpoch {
            version: Version,
            sender_nonce: generation.local,
            recipient_nonce: generation.remote,
            epoch,
        };
        if let Err(error) = write_json(writer, &message, FrameLimit::Normal).await {
            return frame_error(error);
        }
    }
    let heartbeat = Message::Heartbeat {
        version: Version,
        sender_nonce: generation.local,
        recipient_nonce: generation.remote,
    };
    let mut ticks = interval_at(opened + HEARTBEAT_PERIOD, HEARTBEAT_PERIOD);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let queued = tokio::select! {
            biased;
            _ = ticks.tick() => None,
            queued = next_operation(&mut io) => match queued { Some(queued) => Some(queued), None => return SessionError::Transport },
        };
        if let Err(error) = authority.normal_session_identity(pin) {
            return authority_error(error);
        }
        let message = match &queued {
            Some(queued) => Message::Operation {
                version: Version,
                sender_nonce: generation.local,
                recipient_nonce: generation.remote,
                message: queued.message.clone(),
            },
            None => heartbeat.clone(),
        };
        if let Err(error) = write_json(writer, &message, FrameLimit::Normal).await {
            return frame_error(error);
        }
    }
}

async fn next_operation(
    io: &mut Option<crate::service::network::operations::SessionIo>,
) -> Option<crate::service::network::operations::Queued> {
    let Some(io) = io else {
        return std::future::pending().await;
    };
    tokio::select! { biased; message = io.control.recv() => message, message = io.data.recv() => message }
}

#[derive(Default)]
struct OperationRate(VecDeque<Instant>);

impl OperationRate {
    fn admit(&mut self, now: Instant) -> bool {
        while self
            .0
            .front()
            .is_some_and(|oldest| now.duration_since(*oldest) >= Duration::from_secs(1))
        {
            self.0.pop_front();
        }
        if self.0.len() >= 32 {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

#[derive(Default)]
struct ControlRate(VecDeque<Instant>);

impl ControlRate {
    fn admit(&mut self, now: Instant) -> bool {
        while self
            .0
            .front()
            .is_some_and(|oldest| now.duration_since(*oldest) >= RATE_WINDOW)
        {
            self.0.pop_front();
        }
        if self.0.len() == RATE_LIMIT {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlRate, OperationRate, RATE_LIMIT, RATE_WINDOW};
    use tokio::time::{Duration, Instant};

    #[test]
    fn operation_rate_caps_every_sliding_second_without_accumulating_burst_credit() {
        let now = Instant::now();
        let mut rate = OperationRate::default();
        for _ in 0..32 {
            assert!(rate.admit(now));
        }
        for _ in 0..100 {
            assert!(!rate.admit(now + Duration::from_millis(999)));
        }
        assert_eq!(rate.0.len(), 32);
        assert!(rate.admit(now + Duration::from_secs(1)));
        assert_eq!(rate.0.len(), 1);
    }

    #[test]
    fn control_rate_uses_a_bounded_sliding_monotonic_window() {
        let start = Instant::now();
        let mut rate = ControlRate::default();
        for index in 0..RATE_LIMIT {
            assert!(rate.admit(start), "{index}");
        }
        assert!(!rate.admit(start));
        assert!(!rate.admit(start + RATE_WINDOW - Duration::from_nanos(1)));
        assert!(rate.admit(start + RATE_WINDOW));
        assert_eq!(rate.0.len(), 1);
        for index in 1..RATE_LIMIT {
            assert!(
                rate.admit(start + RATE_WINDOW + Duration::from_secs(1)),
                "{index}"
            );
        }
        assert!(!rate.admit(start + RATE_WINDOW + Duration::from_secs(2)));
        assert!(rate.admit(start + RATE_WINDOW * 2));
        assert!(!rate.admit(start + RATE_WINDOW * 2));
    }
}
