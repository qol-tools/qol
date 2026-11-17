#[cfg(test)]
mod tests;

use crate::service::PeerAuthority;
use crate::{
    operations::{Failure, Invocation, OperationEpoch, Outcome, RequestHandle},
    session::AuthenticatedSession,
    PeerId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{mpsc as blocking, Arc, Mutex},
};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

pub trait OperationDispatcher: Send + Sync {
    fn invoke(
        &self,
        authority: &PeerAuthority,
        session: AuthenticatedSession,
        invocation: &Invocation,
    ) -> Result<Outcome, Failure>;
}

struct Unsupported;
impl OperationDispatcher for Unsupported {
    fn invoke(
        &self,
        _: &PeerAuthority,
        _: AuthenticatedSession,
        _: &Invocation,
    ) -> Result<Outcome, Failure> {
        Err(Failure::Unsupported)
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum OperationMessage {
    Invoke {
        invocation: Invocation,
    },
    Outcome {
        handle: RequestHandle,
    },
    Cancel {
        handle: RequestHandle,
    },
    Reply {
        handle: RequestHandle,
        result: Result<Outcome, Failure>,
    },
}

pub(crate) struct Queued {
    pub message: OperationMessage,
    _bytes: OwnedSemaphorePermit,
}

pub(crate) struct SessionIo {
    pub control: mpsc::Receiver<Queued>,
    pub data: mpsc::Receiver<Queued>,
}

struct Route {
    session: AuthenticatedSession,
    epoch_ready: bool,
    control: mpsc::Sender<Queued>,
    data: mpsc::Sender<Queued>,
}

pub(crate) struct Worker {
    pub session: AuthenticatedSession,
    pub invocation: Invocation,
    _slot: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

pub(crate) struct Completion {
    pub worker: Worker,
    pub result: Result<Outcome, Failure>,
}

type Reply = Result<Outcome, Failure>;
type Waiter = (RequestHandle, blocking::SyncSender<Reply>);

pub(crate) struct Hub {
    authority: PeerAuthority,
    dispatcher: Arc<dyn OperationDispatcher>,
    routes: Mutex<BTreeMap<PeerId, Route>>,
    busy: Mutex<BTreeMap<PeerId, RequestHandle>>,
    waiters: Mutex<BTreeMap<PeerId, Waiter>>,
    bytes: Arc<Semaphore>,
    slots: Arc<Semaphore>,
    workers: mpsc::Sender<Worker>,
    stopped: std::sync::atomic::AtomicBool,
}

impl Hub {
    pub fn new(
        authority: PeerAuthority,
        dispatcher: Option<Arc<dyn OperationDispatcher>>,
    ) -> (Arc<Self>, mpsc::Receiver<Worker>) {
        let (workers, incoming) = mpsc::channel(16);
        (
            Arc::new(Self {
                authority,
                dispatcher: dispatcher.unwrap_or_else(|| Arc::new(Unsupported)),
                routes: Mutex::new(BTreeMap::new()),
                busy: Mutex::new(BTreeMap::new()),
                waiters: Mutex::new(BTreeMap::new()),
                bytes: Arc::new(Semaphore::new(1024 * 1024)),
                slots: Arc::new(Semaphore::new(16)),
                workers,
                stopped: std::sync::atomic::AtomicBool::new(false),
            }),
            incoming,
        )
    }

    pub fn elect(&self, session: AuthenticatedSession) -> Result<SessionIo, Failure> {
        let mut routes = self.routes.lock().map_err(|_| Failure::Unavailable)?;
        if self.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Failure::Unavailable);
        }
        self.authority.elect_operation_session(session)?;
        let (control, control_rx) = mpsc::channel(8);
        let (data, data_rx) = mpsc::channel(16);
        routes.insert(
            session.remote_peer,
            Route {
                session,
                epoch_ready: false,
                control,
                data,
            },
        );
        Ok(SessionIo {
            control: control_rx,
            data: data_rx,
        })
    }

    pub fn close(&self, session: AuthenticatedSession) {
        let Ok(mut routes) = self.routes.lock() else {
            return;
        };
        self.authority.close_operation_session(session);
        if routes
            .get(&session.remote_peer)
            .is_some_and(|route| route.session == session)
        {
            routes.remove(&session.remote_peer);
            drop(routes);
            if let Ok(mut waiters) = self.waiters.lock() {
                waiters.remove(&session.remote_peer);
            }
        }
    }

    pub fn epoch(
        &self,
        session: AuthenticatedSession,
        epoch: OperationEpoch,
    ) -> Result<(), Failure> {
        let mut routes = self.routes.lock().map_err(|_| Failure::Unavailable)?;
        let route = routes
            .get_mut(&session.remote_peer)
            .filter(|route| route.session == session)
            .ok_or(Failure::StaleSession)?;
        if route.epoch_ready {
            return Err(Failure::StaleSession);
        }
        self.authority.observe_operation_epoch(session, epoch)?;
        route.epoch_ready = true;
        Ok(())
    }

    pub fn request(
        &self,
        peer: PeerId,
        message: OperationMessage,
    ) -> Result<blocking::Receiver<Reply>, Failure> {
        let handle = match &message {
            OperationMessage::Invoke { invocation } => &invocation.handle,
            OperationMessage::Outcome { handle } | OperationMessage::Cancel { handle } => handle,
            OperationMessage::Reply { .. } => return Err(Failure::Unsupported),
        };
        let mut waiters = self.waiters.lock().map_err(|_| Failure::Unavailable)?;
        if waiters
            .get(&peer)
            .is_some_and(|(pending, _)| pending != handle)
        {
            return Err(Failure::Busy);
        }
        let (send, receive) = blocking::sync_channel(1);
        waiters.insert(peer, (handle.clone(), send));
        if let Err(error) = self.send(peer, None, message) {
            waiters.remove(&peer);
            return Err(error);
        }
        Ok(receive)
    }

    fn send(
        &self,
        peer: PeerId,
        session: Option<AuthenticatedSession>,
        message: OperationMessage,
    ) -> Result<(), Failure> {
        let size = serde_json::to_vec(&message)
            .map_err(|_| Failure::InvalidBody)?
            .len()
            + 256;
        let bytes = self
            .bytes
            .clone()
            .try_acquire_many_owned(size as u32)
            .map_err(|_| Failure::Capacity)?;
        let routes = self.routes.lock().map_err(|_| Failure::Unavailable)?;
        let route = routes
            .get(&peer)
            .filter(|route| {
                route.epoch_ready && session.is_none_or(|session| route.session == session)
            })
            .ok_or(Failure::Unavailable)?;
        let target = if matches!(message, OperationMessage::Invoke { .. }) {
            &route.data
        } else {
            &route.control
        };
        target
            .try_send(Queued {
                message,
                _bytes: bytes,
            })
            .map_err(|_| Failure::Busy)
    }

    pub fn receive(
        &self,
        session: AuthenticatedSession,
        message: OperationMessage,
    ) -> Result<(), Failure> {
        {
            let routes = self.routes.lock().map_err(|_| Failure::Unavailable)?;
            if !routes
                .get(&session.remote_peer)
                .is_some_and(|route| route.session == session && route.epoch_ready)
            {
                return Err(Failure::StaleSession);
            }
        }
        match message {
            OperationMessage::Invoke { invocation } => {
                let result = self.enqueue(session, invocation.clone());
                if let Some(result) = result {
                    self.reply(session, invocation.handle, result)?;
                }
            }
            OperationMessage::Outcome { handle } => {
                let outcome = self.authority.operation_outcome(session, &handle);
                self.reply(session, handle, outcome)?;
            }
            OperationMessage::Cancel { handle } => {
                let outcome = self.authority.cancel_operation(session, &handle);
                self.reply(session, handle, outcome)?;
            }
            OperationMessage::Reply { handle, result } => {
                if let Ok(outcome) = &result {
                    self.authority
                        .record_operation_reply(session, &handle, outcome.clone())?;
                }
                let mut waiters = self.waiters.lock().map_err(|_| Failure::Unavailable)?;
                if waiters
                    .get(&session.remote_peer)
                    .is_some_and(|(expected, _)| *expected == handle)
                {
                    if let Some((_, waiter)) = waiters.remove(&session.remote_peer) {
                        let _ = waiter.try_send(result);
                    }
                }
            }
        }
        Ok(())
    }

    fn enqueue(&self, session: AuthenticatedSession, invocation: Invocation) -> Option<Reply> {
        if crate::service::decode_body(&invocation.body).is_err() {
            return Some(Err(Failure::InvalidBody));
        }
        if crate::service::body_digest(&invocation.body) != invocation.handle.body_digest {
            return Some(Err(Failure::Conflict));
        }
        match self
            .authority
            .operation_outcome(session, &invocation.handle)
        {
            Ok(outcome) => return Some(Ok(outcome)),
            Err(Failure::Gap) => {}
            Err(error) => return Some(Err(error)),
        }
        let mut busy = match self.busy.lock() {
            Ok(busy) => busy,
            Err(_) => return Some(Err(Failure::Unavailable)),
        };
        if let Some(current) = busy.get(&session.remote_peer) {
            if *current == invocation.handle {
                return None;
            }
            return Some(Err(Failure::Busy));
        }
        let Ok(slot) = self.slots.clone().try_acquire_owned() else {
            return Some(Err(Failure::Busy));
        };
        let size = serde_json::to_vec(&invocation)
            .map(|bytes| bytes.len() + 256)
            .unwrap_or(1024 * 1024);
        let Ok(bytes) = self.bytes.clone().try_acquire_many_owned(size as u32) else {
            return Some(Err(Failure::Capacity));
        };
        busy.insert(session.remote_peer, invocation.handle.clone());
        if self
            .workers
            .try_send(Worker {
                session,
                invocation,
                _slot: slot,
                _bytes: bytes,
            })
            .is_err()
        {
            busy.remove(&session.remote_peer);
            return Some(Err(Failure::Busy));
        }
        None
    }

    pub fn execute(&self, worker: Worker) -> Completion {
        let result = self
            .dispatcher
            .invoke(&self.authority, worker.session, &worker.invocation);
        Completion { worker, result }
    }

    pub fn completed(&self, completion: Completion) -> Result<(), crate::network::NetworkFailure> {
        let worker = completion.worker;
        if let Ok(mut busy) = self.busy.lock() {
            busy.remove(&worker.session.remote_peer);
        }
        let failed = completion.result == Err(Failure::Storage);
        let _ = self.reply(worker.session, worker.invocation.handle, completion.result);
        if failed {
            return Err(crate::network::NetworkFailure::Cleanup);
        }
        Ok(())
    }

    fn reply(
        &self,
        session: AuthenticatedSession,
        handle: RequestHandle,
        result: Reply,
    ) -> Result<(), Failure> {
        self.send(
            session.remote_peer,
            Some(session),
            OperationMessage::Reply { handle, result },
        )
    }
}

impl Hub {
    pub fn ready(&self, peer: PeerId) -> bool {
        self.routes
            .lock()
            .is_ok_and(|routes| routes.get(&peer).is_some_and(|route| route.epoch_ready))
    }
}

impl Hub {
    pub fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut routes) = self.routes.lock() {
            for route in routes.values() {
                self.authority.close_operation_session(route.session);
            }
            routes.clear();
        }
        if let Ok(mut waiters) = self.waiters.lock() {
            waiters.clear();
        }
    }
}
