use std::process::Command;
use std::sync::mpsc::channel;

use ratatui::text::Span;

use crate::commands::emu::{ImageCandidate, ResolveState};
use crate::dev_server::WorkspacePlugin;
use qol_dev_env::{
    BootDefinition, EnvironmentDefinition, EnvironmentSnapshot, ImageDefinition, Inventory,
    MountDefinition, ResolvedEnvironment,
};

use super::dash::Dash;
use super::draw::draw;
use super::filters::{FilterStrategy, LogFilter};
use super::log_pane::LogPane;

pub(super) fn span_text(spans: &[Span<'static>]) -> String {
    spans.iter().map(|span| span.content.as_ref()).collect()
}

pub(super) fn log_filter(strategy: FilterStrategy, text: &str) -> LogFilter {
    LogFilter {
        strategy,
        text: text.to_string(),
    }
}

pub(super) fn set_active_filters(dash: &mut Dash, filters: Vec<LogFilter>) {
    let view = dash.view;
    *dash
        .filters
        .for_view_mut(view)
        .expect("active view is filterable") = filters;
}

pub(super) fn workspace_plugin(name: &str, linked: bool, needs_rebuild: bool) -> WorkspacePlugin {
    WorkspacePlugin {
        id: name.to_string(),
        name: name.to_string(),
        version: if linked { "1.0.0" } else { "" }.to_string(),
        path: format!("/ws/{name}"),
        linked,
        needs_rebuild,
        rebuild_reason: if needs_rebuild { "Source changed" } else { "" }.to_string(),
    }
}

pub(super) fn emu_env(id: &str, state: ResolveState) -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        resolved: ResolvedEnvironment {
            definition: EnvironmentDefinition {
                id: id.to_string(),
                name: id.to_string(),
                family: "linux".to_string(),
                backend: "qemu".to_string(),
                image: ImageDefinition {
                    kind: "qcow2".to_string(),
                    base: format!("{id}.qcow2").into(),
                    recommended_size_gb: 16,
                    arch: Some("x86_64".to_string()),
                    firmware: Some("bios".to_string()),
                },
                boot: BootDefinition {
                    memory_mb: 1024,
                    cpus: 1,
                    display: "gtk".to_string(),
                },
                mounts: MountDefinition { workspace: false },
                capabilities: Default::default(),
                source: format!("flows/envs/{id}.toml").into(),
            },
            state,
            image_path: Some(format!("/a/b/{id}.qcow2").into()),
            verified_image: None,
            run_root: Some("/runs".into()),
            messages: Vec::new(),
        },
        runs: Vec::new(),
    }
}

pub(super) fn emu_inventory(environments: Vec<EnvironmentSnapshot>) -> Inventory {
    Inventory {
        environments,
        flows: Vec::new(),
        unassigned_runs: Vec::new(),
        issues: Vec::new(),
    }
}

pub(super) fn emu_candidate(id: &str) -> ImageCandidate {
    use crate::commands::emu::{ArchGuess, BootMedia, Firmware, GuestArch};
    ImageCandidate {
        id: id.to_string(),
        path: std::path::PathBuf::from(format!("/a/b/{id}.qcow2")),
        display_name: id.to_string(),
        arch: ArchGuess::assumed(GuestArch::X86_64),
        firmware: Firmware::Uefi,
        media: BootMedia::Disk,
    }
}

pub(super) fn known_emu_candidate(id: &str) -> ImageCandidate {
    use crate::commands::emu::{ArchGuess, GuestArch};
    let mut candidate = emu_candidate(id);
    candidate.arch = ArchGuess::known(GuestArch::X86_64);
    candidate
}

pub(super) fn live_pane(line: &str) -> LogPane {
    let mut pane = LogPane::new();
    let child = Command::new("true").spawn().unwrap();
    let (_tx, rx) = channel::<String>();
    pane.attach(child, rx);
    pane.push(line.to_string());
    pane
}

pub(super) fn render_text(dash: &mut Dash) -> String {
    use ratatui::backend::TestBackend;
    let mut terminal = ratatui::Terminal::new(TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| draw(frame, dash)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

pub(super) fn render_rows(dash: &mut Dash) -> Vec<String> {
    use ratatui::backend::TestBackend;
    let mut terminal = ratatui::Terminal::new(TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| draw(frame, dash)).unwrap();
    let backend = terminal.backend();
    let buffer = backend.buffer();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

pub(super) fn row_bounds(rows: &[String], needle: &str) -> (usize, usize) {
    let row = rows
        .iter()
        .find(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not rendered"));
    let start = row.find(needle).expect("needle already found");
    let left = row[..start].rfind('│').expect("missing left border");
    let right = row[start..]
        .find('│')
        .map(|index| start + index)
        .expect("missing right border");
    (left, right)
}

pub(super) fn attach_lines(lines: &[&str]) -> LogPane {
    let mut pane = LogPane::collapsing();
    let child = Command::new("true").spawn().unwrap();
    let (tx, rx) = channel::<String>();
    for line in lines {
        tx.send((*line).to_string()).unwrap();
    }
    pane.attach(child, rx);
    pane
}
