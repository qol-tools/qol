mod graph;

use std::fs::File;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::platform::{CaptureProcess, CaptureSession};
use crate::Config;
use graph::Graph;

const SINK: &str = "qol_shot_apps";
const SINK_DESCRIPTION: &str = "QoL Shot app audio";
const KEEPER_ENV: &str = "QOL_SHOT_APP_AUDIO_REQUEST";
const SINK_READY_TIMEOUT: Duration = Duration::from_secs(2);
const SINK_POLL_INTERVAL: Duration = Duration::from_millis(40);
const KEEP_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Deserialize, Serialize)]
struct KeeperRequest {
    apps: Vec<String>,
    sink: u64,
    processes: Vec<CaptureProcess>,
}

pub(super) fn records_apps(config: &Config) -> bool {
    let chosen = |input: &str| config.audio.inputs.iter().any(|value| value == input);
    config.audio.enabled && !chosen("system") && !config.audio.apps.is_empty()
}

pub(super) fn monitor_source() -> String {
    format!("{SINK}.monitor")
}

pub(super) fn playing_apps() -> Vec<String> {
    match graph() {
        Ok(graph) => graph.playing_apps(),
        Err(error) => {
            log::warn!("listing the apps playing sound failed: {error:#}");
            Vec::new()
        }
    }
}

pub(super) struct Tap {
    apps: Vec<String>,
    sink: Option<u64>,
}

impl Tap {
    pub(super) fn open(config: &Config) -> Result<Option<Self>> {
        if !records_apps(config) {
            return Ok(None);
        }
        let mut tap = Self {
            apps: config.audio.apps.clone(),
            sink: None,
        };
        remove_stale_sinks()?;
        run("pw-cli", &["create-node", "adapter", &sink_props()])?;
        let opened = wait_for_sink().and_then(|(sink, serial)| {
            tap.sink = Some(serial);
            link_missing(&tap.apps, sink)
        });
        match opened {
            Ok(linked) => {
                qol_runtime::probe!(
                    "SHOT_RECORD_APP_AUDIO",
                    "step=open apps={} linked={linked}",
                    tap.apps.len()
                );
                Ok(Some(tap))
            }
            Err(error) => {
                tap.close();
                Err(error)
            }
        }
    }

    pub(super) fn keep(self, session: &CaptureSession) {
        if let Err(error) = self.spawn_keeper(session) {
            log::warn!("app audio streams opened during the recording stay out of it: {error:#}");
        }
    }

    fn spawn_keeper(&self, session: &CaptureSession) -> Result<()> {
        let sink = self
            .sink
            .ok_or_else(|| anyhow!("the app audio sink has no serial"))?;
        let request = KeeperRequest {
            apps: self.apps.clone(),
            sink,
            processes: session.processes.clone(),
        };
        let request = serde_json::to_string(&request).context("failed to encode the request")?;
        let log_file = File::options()
            .create(true)
            .append(true)
            .open(super::super::CAPTURE_LOG)
            .context("failed to open the recording log")?;
        let executable = std::env::current_exe().context("failed to resolve qol-shot")?;
        let mut keeper = Command::new(executable)
            .env(KEEPER_ENV, request)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log_file))
            .spawn()
            .context("failed to start the app audio keeper")?;
        std::thread::Builder::new()
            .name("qol-shot-app-audio-reaper".to_string())
            .spawn(move || keeper.wait())
            .context("failed to watch the app audio keeper")?;
        Ok(())
    }

    pub(super) fn close(self) {
        let Some(serial) = self.sink else {
            if let Err(error) = remove_stale_sinks() {
                log::warn!("the app audio sink {SINK} stays until the next recording: {error:#}");
            }
            return;
        };
        if let Err(error) = remove_sink(serial) {
            log::warn!("the app audio sink {SINK} stays until the next recording: {error:#}");
        }
    }
}

pub(super) fn run_internal_helper() -> Option<ExitCode> {
    let request = std::env::var(KEEPER_ENV).ok()?;
    Some(match keep_linked(&request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("app audio keeper failed: {error:#}");
            ExitCode::FAILURE
        }
    })
}

fn keep_linked(request: &str) -> Result<()> {
    let request: KeeperRequest =
        serde_json::from_str(request).context("invalid app audio request")?;
    let stop = qol_process::CancellationToken::install()
        .context("failed to listen for the stop request")?;
    while !stop.is_cancelled() && request.processes.iter().any(recording) {
        let graph = match graph() {
            Ok(graph) => graph,
            Err(error) => {
                log::warn!("reading the PipeWire graph failed: {error:#}");
                std::thread::sleep(KEEP_INTERVAL);
                continue;
            }
        };
        let Some(sink) = graph.node_with_serial(request.sink) else {
            bail!("the app audio sink {SINK} was removed during the recording");
        };
        let linked = link_graph(&graph, &request.apps, sink);
        if linked > 0 {
            qol_runtime::probe!("SHOT_RECORD_APP_AUDIO", "step=keep linked={linked}");
        }
        std::thread::sleep(KEEP_INTERVAL);
    }
    qol_runtime::probe!("SHOT_RECORD_APP_AUDIO", "step=close");
    remove_sink(request.sink)
}

fn recording(process: &CaptureProcess) -> bool {
    process.alive() && !qol_process::is_pid_zombie(process.pid)
}

fn sink_props() -> String {
    format!(
        "{{ factory.name=support.null-audio-sink node.name={SINK} node.description=\"{SINK_DESCRIPTION}\" media.class=Audio/Sink audio.position=[ FL FR ] object.linger=true node.virtual=true priority.session=0 priority.driver=0 }}"
    )
}

fn wait_for_sink() -> Result<(u32, u64)> {
    let deadline = Instant::now() + SINK_READY_TIMEOUT;
    loop {
        let graph = graph()?;
        let ready = graph
            .node_named(SINK)
            .filter(|sink| graph.has_inputs(*sink))
            .and_then(|sink| Some((sink, graph.serial_of(sink)?)));
        if let Some(ready) = ready {
            return Ok(ready);
        }
        if Instant::now() >= deadline {
            bail!("PipeWire did not create the app audio sink {SINK}");
        }
        std::thread::sleep(SINK_POLL_INTERVAL);
    }
}

fn link_missing(apps: &[String], sink: u32) -> Result<usize> {
    Ok(link_graph(&graph()?, apps, sink))
}

fn link_graph(graph: &Graph, apps: &[String], sink: u32) -> usize {
    graph
        .missing_links(apps, sink)
        .into_iter()
        .filter(|(output, input)| {
            match run("pw-link", &[&output.to_string(), &input.to_string()]) {
                Ok(_) => true,
                Err(error) => {
                    log::warn!("app audio port {output} stays out of the recording: {error:#}");
                    false
                }
            }
        })
        .count()
}

fn remove_stale_sinks() -> Result<()> {
    for sink in graph()?.nodes_named(SINK) {
        run("pw-cli", &["destroy", &sink.to_string()])?;
    }
    Ok(())
}

fn remove_sink(serial: u64) -> Result<()> {
    if let Some(sink) = graph()?.node_with_serial(serial) {
        run("pw-cli", &["destroy", &sink.to_string()])?;
    }
    Ok(())
}

fn graph() -> Result<Graph> {
    Graph::parse(&run("pw-dump", &[])?).context("pw-dump did not print the PipeWire graph")
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("recording app audio needs PipeWire's {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::{records_apps, sink_props};
    use crate::Config;

    #[test]
    fn apps_are_recorded_when_named_and_not_covered_by_system_audio() {
        let cases: [(bool, &[&str], &[&str], bool); 6] = [
            (true, &[], &["KCD2"], true),
            (true, &["mic"], &["KCD2"], true),
            (true, &["mic"], &[], false),
            (true, &["system"], &["KCD2"], false),
            (true, &["mic", "system"], &["KCD2"], false),
            (false, &["mic"], &["KCD2"], false),
        ];
        for (enabled, inputs, apps, expected) in cases {
            let mut config = Config::default();
            config.audio.enabled = enabled;
            config.audio.inputs = inputs.iter().map(|input| (*input).to_string()).collect();
            config.audio.apps = apps.iter().map(|app| (*app).to_string()).collect();
            assert_eq!(
                records_apps(&config),
                expected,
                "{enabled} {inputs:?} {apps:?}"
            );
        }
    }

    #[test]
    fn sink_never_competes_for_the_default_output_or_the_clock() {
        let props = sink_props();
        for prop in [
            "factory.name=support.null-audio-sink",
            "node.name=qol_shot_apps",
            "media.class=Audio/Sink",
            "priority.session=0",
            "priority.driver=0",
            "object.linger=true",
        ] {
            assert!(props.contains(prop), "{props}");
        }
    }
}
