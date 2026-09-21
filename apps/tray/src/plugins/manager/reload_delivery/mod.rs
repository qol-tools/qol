use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) const RELOAD_DELIVERY_TTL: Duration = Duration::from_secs(15);

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DaemonInstance {
    pub(crate) pid: u32,
    pub(crate) incarnation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingReloadTicket {
    plugin_id: String,
    request_id: u64,
    daemon_instance: Option<DaemonInstance>,
}

impl PendingReloadTicket {
    pub(crate) fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub(crate) fn request_id(&self) -> u64 {
        self.request_id
    }

    pub(crate) fn daemon_instance(&self) -> Option<DaemonInstance> {
        self.daemon_instance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReloadDeliveryOutcome {
    Handled,
    Failed,
    NoDaemon,
    SaveFailed,
    SnapshotFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliverySettlement {
    Acknowledged { generation: u64 },
    Superseded,
    Restart { generation: Option<u64> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompletionDecision {
    Acknowledged { generation: u64 },
    Superseded,
    Restarted,
    RestartFailed(String),
}

struct PendingReloadRequest {
    request_id: u64,
    daemon_instance: Option<DaemonInstance>,
    generation: Option<u64>,
    registered_at: Instant,
}

#[derive(Default)]
pub(crate) struct ReloadDeliveries {
    by_plugin: HashMap<String, Vec<PendingReloadRequest>>,
}

impl ReloadDeliveries {
    pub(crate) fn begin(
        &mut self,
        plugin_id: &str,
        daemon_instance: Option<DaemonInstance>,
        now: Instant,
    ) -> PendingReloadTicket {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        self.by_plugin
            .entry(plugin_id.to_string())
            .or_default()
            .push(PendingReloadRequest {
                request_id,
                daemon_instance,
                generation: None,
                registered_at: now,
            });
        qol_runtime::probe!(
            "PLUGIN_RELOAD",
            "plugin={plugin_id} stage=register scope=single request_id={request_id} daemon_instance={} published_generation=none acknowledged_generation=none",
            instance_label(daemon_instance)
        );
        PendingReloadTicket {
            plugin_id: plugin_id.to_string(),
            request_id,
            daemon_instance,
        }
    }

    pub(crate) fn record_saved(&mut self, ticket: &PendingReloadTicket, generation: u64) -> bool {
        let Some(requests) = self.by_plugin.get_mut(&ticket.plugin_id) else {
            return false;
        };
        let Some(request) = requests
            .iter_mut()
            .find(|request| request.request_id == ticket.request_id)
        else {
            return false;
        };
        if request.daemon_instance != ticket.daemon_instance {
            return false;
        }
        request.generation = Some(generation);
        qol_runtime::probe!(
            "PLUGIN_RELOAD",
            "plugin={} stage=saved scope=single request_id={} daemon_instance={} published_generation={generation} acknowledged_generation=none",
            ticket.plugin_id,
            ticket.request_id,
            instance_label(ticket.daemon_instance)
        );
        true
    }

    pub(crate) fn is_pending(&self, plugin_id: &str) -> bool {
        self.by_plugin
            .get(plugin_id)
            .is_some_and(|requests| !requests.is_empty())
    }

    pub(crate) fn complete(
        &mut self,
        ticket: &PendingReloadTicket,
        outcome: ReloadDeliveryOutcome,
        current_instance: Option<DaemonInstance>,
    ) -> DeliverySettlement {
        let Some(requests) = self.by_plugin.get_mut(&ticket.plugin_id) else {
            return DeliverySettlement::Superseded;
        };
        let Some(index) = requests
            .iter()
            .position(|request| request.request_id == ticket.request_id)
        else {
            return DeliverySettlement::Superseded;
        };
        if requests[index].daemon_instance != current_instance {
            let request = requests.remove(index);
            qol_runtime::probe!(
                "PLUGIN_RELOAD",
                "plugin={} stage=complete decision=superseded reason=daemon-replaced request_id={} published_generation={} acknowledged_generation=none",
                ticket.plugin_id,
                ticket.request_id,
                generation_label(request.generation)
            );
            return DeliverySettlement::Superseded;
        }
        let concurrent_pending = requests.len() > 1;
        let request = requests.remove(index);
        let decision = match outcome {
            ReloadDeliveryOutcome::Handled => match request.generation {
                Some(generation) => {
                    qol_runtime::probe!(
                        "PLUGIN_RELOAD",
                        "plugin={} stage=complete decision=acknowledged request_id={} published_generation={generation} acknowledged_generation={generation}",
                        ticket.plugin_id,
                        ticket.request_id
                    );
                    DeliverySettlement::Acknowledged { generation }
                }
                None => {
                    qol_runtime::probe!(
                        "PLUGIN_RELOAD",
                        "plugin={} stage=complete decision=superseded reason=unpublished request_id={} published_generation=none acknowledged_generation=none",
                        ticket.plugin_id,
                        ticket.request_id
                    );
                    DeliverySettlement::Superseded
                }
            },
            ReloadDeliveryOutcome::Failed if concurrent_pending => {
                qol_runtime::probe!(
                    "PLUGIN_RELOAD",
                    "plugin={} stage=complete decision=superseded reason=delivery-pending request_id={} published_generation={} acknowledged_generation=none",
                    ticket.plugin_id,
                    ticket.request_id,
                    generation_label(request.generation)
                );
                DeliverySettlement::Superseded
            }
            ReloadDeliveryOutcome::Failed => {
                qol_runtime::probe!(
                    "PLUGIN_RELOAD",
                    "plugin={} stage=complete decision=restart reason=transport-failure request_id={} published_generation={} acknowledged_generation=none",
                    ticket.plugin_id,
                    ticket.request_id,
                    generation_label(request.generation)
                );
                DeliverySettlement::Restart {
                    generation: request.generation,
                }
            }
            ReloadDeliveryOutcome::NoDaemon => {
                qol_runtime::probe!(
                    "PLUGIN_RELOAD",
                    "plugin={} stage=complete decision=superseded reason=no-daemon request_id={} published_generation={} acknowledged_generation=none",
                    ticket.plugin_id,
                    ticket.request_id,
                    generation_label(request.generation)
                );
                DeliverySettlement::Superseded
            }
            ReloadDeliveryOutcome::SaveFailed => {
                qol_runtime::probe!(
                    "PLUGIN_RELOAD",
                    "plugin={} stage=complete decision=superseded reason=save-failed request_id={} published_generation={} acknowledged_generation=none",
                    ticket.plugin_id,
                    ticket.request_id,
                    generation_label(request.generation)
                );
                DeliverySettlement::Superseded
            }
            ReloadDeliveryOutcome::SnapshotFailed => {
                qol_runtime::probe!(
                    "PLUGIN_RELOAD",
                    "plugin={} stage=complete decision=superseded reason=snapshot-failed request_id={} published_generation={} acknowledged_generation=none",
                    ticket.plugin_id,
                    ticket.request_id,
                    generation_label(request.generation)
                );
                DeliverySettlement::Superseded
            }
        };
        self.prune_empty(&ticket.plugin_id);
        decision
    }

    pub(crate) fn expire_stale(&mut self, now: Instant, ttl: Duration) -> usize {
        let plugin_ids = self.by_plugin.keys().cloned().collect::<Vec<_>>();
        let mut expired = 0;
        for plugin_id in plugin_ids {
            {
                let Some(requests) = self.by_plugin.get_mut(&plugin_id) else {
                    continue;
                };
                let mut index = 0;
                while index < requests.len() {
                    if now.saturating_duration_since(requests[index].registered_at) < ttl {
                        index += 1;
                        continue;
                    }
                    let request = requests.remove(index);
                    qol_runtime::probe!(
                        "PLUGIN_RELOAD",
                        "plugin={plugin_id} stage=expire scope=single request_id={} daemon_instance={} published_generation={} acknowledged_generation=none",
                        request.request_id,
                        instance_label(request.daemon_instance),
                        generation_label(request.generation)
                    );
                    expired += 1;
                }
            }
            if self.by_plugin.get(&plugin_id).is_some_and(Vec::is_empty) {
                self.by_plugin.remove(&plugin_id);
            }
        }
        expired
    }

    pub(crate) fn supersede_plugin(&mut self, plugin_id: &str) {
        let Some(requests) = self.by_plugin.remove(plugin_id) else {
            return;
        };
        for request in requests {
            qol_runtime::probe!(
                "PLUGIN_RELOAD",
                "plugin={plugin_id} stage=supersede scope=single request_id={} daemon_instance={} published_generation={} acknowledged_generation=none",
                request.request_id,
                instance_label(request.daemon_instance),
                generation_label(request.generation)
            );
        }
    }

    pub(crate) fn supersede_all(&mut self) {
        let plugin_ids = self.by_plugin.keys().cloned().collect::<Vec<_>>();
        for plugin_id in plugin_ids {
            self.supersede_plugin(&plugin_id);
        }
    }

    fn prune_empty(&mut self, plugin_id: &str) {
        if self.by_plugin.get(plugin_id).is_some_and(Vec::is_empty) {
            self.by_plugin.remove(plugin_id);
        }
    }
}

pub(crate) fn now() -> Instant {
    #[cfg(test)]
    {
        if let Some(instant) = test_now() {
            return instant;
        }
    }
    Instant::now()
}

pub(crate) fn instance_label(instance: Option<DaemonInstance>) -> String {
    match instance {
        Some(instance) => format!("{}:{}", instance.pid, instance.incarnation),
        None => "none".to_string(),
    }
}

pub(crate) fn generation_label(generation: Option<u64>) -> String {
    match generation {
        Some(generation) => generation.to_string(),
        None => "none".to_string(),
    }
}

#[cfg(test)]
thread_local! {
    static TEST_NOW: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn test_now() -> Option<Instant> {
    TEST_NOW.with(std::cell::Cell::get)
}

#[cfg(all(test, unix))]
pub(crate) fn set_test_now(instant: Option<Instant>) {
    TEST_NOW.with(|cell| cell.set(instant));
}

#[cfg(test)]
mod tests;
