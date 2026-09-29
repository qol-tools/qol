mod admission;
mod lifecycle;
mod operations;
mod partial;
mod wire;

use std::{
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::{Duration, SystemTime},
};

use tokio::{
    io::{duplex, AsyncRead, AsyncWrite, DuplexStream, ReadBuf},
    time::timeout,
};

use crate::{
    service::{
        framing::{read_json, write_json, FrameLimit},
        PeerAuthority, PeerConnection,
    },
    session::{SessionGeneration, SessionNonce},
};

use super::{
    wire::{Message, Version},
    NormalSession, SessionError, SessionOutcome,
};

struct Fixture {
    left: PeerAuthority,
    right: PeerAuthority,
    drops: Arc<AtomicUsize>,
    fail_left_write: Arc<AtomicBool>,
    stall_left_write: Arc<AtomicBool>,
}

impl Fixture {
    fn new() -> Self {
        let left = PeerAuthority::session("left".into(), SystemTime::now()).unwrap();
        let right = PeerAuthority::session("right".into(), SystemTime::now()).unwrap();
        link(&left, &right);
        link(&right, &left);
        Self {
            left,
            right,
            drops: Arc::new(AtomicUsize::new(0)),
            fail_left_write: Arc::new(AtomicBool::new(false)),
            stall_left_write: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn connections(
        &self,
        capacity: usize,
    ) -> (PeerConnection<Tracked>, PeerConnection<Tracked>) {
        let client = self
            .left
            .normal_client_config(self.right.local_pin().unwrap().peer_id())
            .unwrap();
        let server = self.right.server_config().unwrap();
        let (left, right) = duplex(capacity);
        let left = Tracked {
            io: left,
            drops: self.drops.clone(),
            fail_write: self.fail_left_write.clone(),
            stall_write: self.stall_left_write.clone(),
        };
        let right = Tracked {
            io: right,
            drops: self.drops.clone(),
            fail_write: Arc::new(AtomicBool::new(false)),
            stall_write: Arc::new(AtomicBool::new(false)),
        };
        let (left, right) = timeout(Duration::from_secs(5), async {
            tokio::join!(client.connect(left), server.accept(right))
        })
        .await
        .unwrap();
        (left.unwrap(), right.unwrap())
    }

    async fn sessions(&self, capacity: usize) -> (NormalSession<Tracked>, NormalSession<Tracked>) {
        let (left, right) = self.connections(capacity).await;
        let (left, right) = tokio::join!(
            NormalSession::open(left, self.left.clone()),
            NormalSession::open(right, self.right.clone()),
        );
        (left.unwrap(), right.unwrap())
    }

    async fn one_session(
        &self,
    ) -> (
        NormalSession<Tracked>,
        PeerConnection<Tracked>,
        SessionGeneration,
    ) {
        let (left, mut right) = self.connections(4096).await;
        let (session, ()) = tokio::join!(NormalSession::open(left, self.left.clone()), async {
            answer_hello(&mut right, &self.right).await;
        });
        let session = session.unwrap();
        let generation = session.authenticated().unwrap().generation;
        (session, right, generation)
    }
}

fn link(local: &PeerAuthority, remote: &PeerAuthority) {
    local
        .insert_link(
            local.projection().unwrap().revision,
            remote.local_pin().unwrap(),
            "peer".into(),
        )
        .unwrap();
}

fn revoke(local: &PeerAuthority, remote: &PeerAuthority) {
    local
        .revoke(
            local.projection().unwrap().revision,
            remote.local_pin().unwrap().peer_id(),
        )
        .unwrap();
}

fn rename(authority: &PeerAuthority) {
    authority
        .rename(authority.projection().unwrap().revision, "renamed".into())
        .unwrap();
}

async fn answer_hello<S: AsyncRead + AsyncWrite + Unpin>(
    io: &mut PeerConnection<S>,
    authority: &PeerAuthority,
) {
    let hello: Message = read_json(io, FrameLimit::Normal).await.unwrap();
    let Message::Hello { sender, .. } = hello else {
        panic!("expected hello")
    };
    write_json(
        io,
        &Message::Hello {
            version: Version,
            sender: authority.local_pin().unwrap().peer_id(),
            recipient: sender,
            nonce: SessionNonce::from_random([7; 16]),
        },
        FrameLimit::Normal,
    )
    .await
    .unwrap();
}

fn heartbeat(generation: SessionGeneration) -> Message {
    Message::Heartbeat {
        version: Version,
        sender_nonce: generation.remote,
        recipient_nonce: generation.local,
    }
}

fn frame(message: &Message) -> Vec<u8> {
    let payload = serde_json::to_vec(message).unwrap();
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(&payload);
    bytes
}

struct Tracked {
    io: DuplexStream,
    drops: Arc<AtomicUsize>,
    fail_write: Arc<AtomicBool>,
    stall_write: Arc<AtomicBool>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AsyncRead for Tracked {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buffer)
    }
}

impl AsyncWrite for Tracked {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::other("write failed")));
        }
        if self.stall_write.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.io).poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
