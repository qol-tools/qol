use std::collections::{HashMap, HashSet};
use std::time::Instant;

use anyhow::Result;
use bluer::{Adapter, Address};
use futures::future::{pending, LocalBoxFuture};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};

use super::{
    apply_report, complete_device_action_within, connect_with, ensure_powered,
    finish_device_action, reconnect_with, redacted, remove_with, schedule_failed,
    trace_manual_failure, trace_remove_action, trace_report, AudioWatchState, ConnectionMode,
    ConnectionSource, ReconnectConfig, ReconnectReport, ReconnectSelection, RetryState,
    EXPLICIT_DEVICE_ACTION_TIMEOUT,
};

pub(super) enum Completion {
    Retry(Address, u32, Result<()>),
    Reconnect(ReconnectSelection, Result<ReconnectReport>),
    Remove(Address, Result<()>),
}

#[derive(Default)]
pub(super) struct DaemonOperations {
    pending: FuturesUnordered<LocalBoxFuture<'static, Completion>>,
    retrying: HashSet<Address>,
}

impl DaemonOperations {
    pub(super) fn retry_deadline(
        &self,
        retries: &HashMap<Address, RetryState>,
        powered: bool,
    ) -> Option<Instant> {
        if !powered {
            return None;
        }
        retries
            .iter()
            .filter(|(address, _)| !self.retrying.contains(address))
            .filter_map(|(_, state)| state.due())
            .min()
    }

    pub(super) fn start_due(&mut self, adapter: &Adapter, retries: &HashMap<Address, RetryState>) {
        let now = Instant::now();
        for (&address, state) in retries {
            if !state.is_due(now) || !self.retrying.insert(address) {
                continue;
            }
            let attempt = state.failures() + 1;
            let adapter = adapter.clone();
            self.pending.push(
                async move {
                    qol_runtime::probe!(
                        "BLUETOOTH_ATTEMPT",
                        "device={} source=auto_retry attempt={attempt}",
                        redacted(address)
                    );
                    let result = complete_device_action_within(
                        "Reconnect",
                        EXPLICIT_DEVICE_ACTION_TIMEOUT,
                        connect_with(
                            &adapter,
                            address,
                            ConnectionMode::Reconnect,
                            ConnectionSource::AutoRetry,
                            None,
                        ),
                    )
                    .await
                    .map(|_| ());
                    Completion::Retry(address, attempt, result)
                }
                .boxed_local(),
            );
        }
    }

    pub(super) fn reconnect(
        &mut self,
        adapter: Adapter,
        config: ReconnectConfig,
        selection: ReconnectSelection,
    ) {
        self.pending.push(
            async move {
                let result = async {
                    ensure_powered(&adapter, config.power_on_adapter).await?;
                    reconnect_with(&adapter, &config, selection).await
                }
                .await;
                Completion::Reconnect(selection, result)
            }
            .boxed_local(),
        );
    }

    pub(super) fn remove(&mut self, adapter: Adapter, address: Address) {
        self.pending.push(
            async move {
                let result = complete_device_action_within(
                    "Remove",
                    EXPLICIT_DEVICE_ACTION_TIMEOUT,
                    remove_with(&adapter, address),
                )
                .await;
                Completion::Remove(address, result)
            }
            .boxed_local(),
        );
    }

    pub(super) async fn next(&mut self) -> Completion {
        if self.pending.is_empty() {
            return pending().await;
        }
        let completion = self
            .pending
            .next()
            .await
            .expect("the operation queue is not empty");
        if let Completion::Retry(address, _, _) = &completion {
            self.retrying.remove(address);
        }
        completion
    }
}

pub(super) fn complete(
    completion: Completion,
    retries: &mut HashMap<Address, RetryState>,
    config: &ReconnectConfig,
    subscribed: &mut HashSet<Address>,
    watch_states: &mut HashMap<Address, AudioWatchState>,
) {
    match completion {
        Completion::Retry(address, attempt, Ok(())) => {
            if let Some(state) = retries.get_mut(&address) {
                state.connected();
            }
            qol_runtime::probe!(
                "BLUETOOTH_RESULT",
                "device={} source=auto_retry outcome=connected attempt={attempt}",
                redacted(address)
            );
        }
        Completion::Retry(address, _, Err(error)) => {
            eprintln!(
                "Bluetooth reconnect failed for {}: {error:#}",
                redacted(address)
            );
            if config.auto_reconnect {
                schedule_failed(retries, &[address], config, Instant::now());
            }
        }
        Completion::Reconnect(selection, Ok(report)) => {
            apply_report(retries, &report, config);
            trace_report(&report, selection);
        }
        Completion::Reconnect(selection, Err(error)) => {
            trace_manual_failure(&error, selection, "reconnect")
        }
        Completion::Remove(address, result) => {
            finish_device_action(address, "Remove", &result);
            if result.is_ok() {
                retries.remove(&address);
                subscribed.remove(&address);
                watch_states.remove(&address);
            }
            trace_remove_action(address, result);
        }
    }
}
