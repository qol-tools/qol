use std::sync::{Arc, Mutex};

use qol_peers::admin::{ActivationId, Error};
use qol_peers::network::NetworkFailure;
use qol_peers::service::network::{
    prepare_with_operations, NetworkOptions, NetworkTask, OperationDispatcher,
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
    task::JoinSet,
};

use super::{ActiveAuthority, Host, PeerHostHandle, State};

pub(super) struct NetworkLauncher {
    pub options: NetworkOptions,
    jobs: mpsc::Sender<Job>,
    dispatcher: Arc<dyn OperationDispatcher>,
}

struct Job {
    activation: ActivationId,
    task: NetworkTask,
}

impl PeerHostHandle {
    pub(super) fn attach_network(&self, runtime: Handle, options: NetworkOptions) {
        let (jobs, incoming) = mpsc::channel(1);
        let inner = self.inner.clone();
        let closing = self.closing.subscribe();
        let mut host = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let dispatcher = Arc::new(super::operations::Dispatcher {
            host: Arc::downgrade(&self.inner),
            plugins: Arc::downgrade(&self.plugins),
        });
        host.network = Some(NetworkLauncher {
            options,
            jobs,
            dispatcher,
        });
        host.driver = Some(runtime.spawn(drive(inner, incoming, closing)));
    }

    pub async fn wait_stopped(&self) -> Result<(), Error> {
        let mut completion = self.completion.subscribe();
        loop {
            {
                let host = self.inner.lock().map_err(|_| Error::HostUnavailable)?;
                match &host.state {
                    State::Stopping(active) if active.cleanup_failed => {
                        return Err(Error::HostUnavailable)
                    }
                    State::Stopping(_) => {}
                    _ => return Ok(()),
                }
            }
            completion
                .changed()
                .await
                .map_err(|_| Error::HostUnavailable)?;
        }
    }

    pub async fn shutdown_and_wait(&self) -> Result<(), Error> {
        self.shutdown();
        let stopped = self.wait_stopped().await;
        let task = self
            .inner
            .lock()
            .map_err(|_| Error::HostUnavailable)?
            .driver
            .take();
        let mut driver = DriverReceipt {
            host: self.inner.clone(),
            task,
        };
        if let Some(task) = driver.task.as_mut() {
            task.await.map_err(|_| Error::HostUnavailable)?;
            driver.task.take();
        }
        stopped
    }
}

struct DriverReceipt {
    host: Arc<Mutex<Host>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for DriverReceipt {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            self.host
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .driver = Some(task);
        }
    }
}

impl Host {
    pub(super) fn activate(&mut self, mut active: ActiveAuthority) {
        if let Some(network) = &self.network {
            let (control, task) = prepare_with_operations(
                active.authority.clone(),
                network.options.clone(),
                network.dispatcher.clone(),
            );
            let job = Job {
                activation: active.activation_id,
                task,
            };
            match network.jobs.try_send(job) {
                Ok(()) => {
                    active.network = Some(control);
                    active.network_complete = false;
                }
                Err(_) => active.network_failure = Some(NetworkFailure::RuntimeUnavailable),
            }
        }
        self.state = State::Active(active);
    }

    pub(super) fn begin_stop(&mut self) {
        let previous = std::mem::replace(&mut self.state, State::Inactive);
        match previous {
            State::Active(active) | State::Stopping(active)
                if !active.network_complete || active.cleanup_failed =>
            {
                if let Some(network) = &active.network {
                    network.stop();
                }
                self.state = State::Stopping(active);
            }
            _ if self.shutting_down => self.state = State::Shutdown,
            _ => {}
        }
    }

    fn network_completed(&mut self, activation: ActivationId, result: Result<(), NetworkFailure>) {
        let stopping = matches!(self.state, State::Stopping(_));
        let active = match &mut self.state {
            State::Active(active) | State::Stopping(active)
                if active.activation_id == activation =>
            {
                active
            }
            _ => return,
        };
        qol_runtime::probe!(
            "TRAY_PEERS",
            "event=network_reaped activation={} outcome={:?}",
            activation,
            result
        );
        active.network_complete = true;
        active.network_failure = result.err();
        active.cleanup_failed = matches!(
            active.network_failure,
            Some(NetworkFailure::Cleanup | NetworkFailure::Task)
        );
        if stopping && !active.cleanup_failed {
            self.state = if self.shutting_down {
                State::Shutdown
            } else {
                State::Inactive
            };
        }
        self.completion
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        super::trace_lifecycle("network_completed", &self.state);
    }
}

async fn drive(
    host: Arc<Mutex<Host>>,
    mut incoming: mpsc::Receiver<Job>,
    mut closing: watch::Receiver<bool>,
) {
    let mut tasks = JoinSet::new();
    let mut activation = None;
    let mut shutdown = false;
    loop {
        if shutdown && tasks.is_empty() && incoming.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            result = tasks.join_next(), if !tasks.is_empty() => {
                let (id, result) = match result {
                    Some(Ok((id, result))) => (Some(id), result),
                    Some(Err(_)) => (activation, Err(NetworkFailure::Task)),
                    None => continue,
                };
                if let Some(id) = id {
                    host.lock().unwrap_or_else(|error| error.into_inner()).network_completed(id, result);
                }
                activation = None;
            },
            _ = closing.changed(), if !shutdown => {
                shutdown = true;
                incoming.close();
            },
            job = incoming.recv(), if tasks.is_empty() && (!shutdown || !incoming.is_empty()) => {
                let Some(job) = job else { shutdown = true; continue; };
                activation = Some(job.activation);
                tasks.spawn(async move { (job.activation, job.task.await) });
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::linked_computers::tests::{attach, authority, host_at};
    use qol_peers::admin::Request;

    #[test]
    fn completion_from_an_old_activation_cannot_change_a_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let host = host_at(&temporary.path().join("peers"), false);
        let shared = attach(&host);
        shared.peer_admin(Request::StartSession {
            name: "first".into(),
        });
        let old = authority(&shared);
        shared.peer_admin(Request::Stop {
            expected: old.expected(),
        });
        shared.peer_admin(Request::StartSession {
            name: "second".into(),
        });
        let current = authority(&shared);
        host.inner
            .lock()
            .unwrap()
            .network_completed(old.activation_id, Err(NetworkFailure::Cleanup));
        assert_eq!(authority(&shared), current);
        assert!(
            !host
                .inner
                .lock()
                .unwrap()
                .authority()
                .unwrap()
                .cleanup_failed
        );
        host.shutdown();
    }
}
