use std::sync::mpsc;

use futures::future::{FutureExt, LocalBoxFuture};
use gpui::AsyncApp;
use qol_gpui::monitor::MonitorTracker;
use qol_headless::CommandResult;

use crate::config::{contract, load_alt_tab_config};
use crate::picker::run::PickerHost;
use crate::PLUGIN_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Operation {
    Daemon,
    Show,
    ShowReverse,
    Settings,
    Kill,
}

pub(super) fn execute(operation: Operation) -> CommandResult {
    match operation {
        Operation::Daemon => run_picker(InitialRequest::Hidden),
        Operation::Show => run_picker(InitialRequest::Show),
        Operation::ShowReverse => run_picker(InitialRequest::ShowReverse),
        Operation::Settings => {
            open_settings_page();
            CommandResult::success("")
        }
        Operation::Kill => {
            super::daemon::send_kill();
            CommandResult::success("")
        }
    }
}

fn open_settings_page() {
    if let Err(error) = qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID) {
        log::warn!("Failed to open settings page: {error}");
    }
}

fn open_settings_panel(cx: AsyncApp, tracker: MonitorTracker) -> LocalBoxFuture<'static, ()> {
    async move {
        let opened = qol_gpui::settings_panel::open_plugin_settings(
            PLUGIN_ID,
            "Alt Tab Settings",
            contract(),
            tracker,
            qol_gpui::settings_panel::SettingsRuntime::empty(),
            &cx,
        )
        .await;
        if let Err(error) = opened {
            log::warn!("settings panel failed, opening browser: {error:#}");
            open_settings_page();
        }
    }
    .boxed_local()
}

#[derive(Clone, Copy)]
enum InitialRequest {
    Hidden,
    Show,
    ShowReverse,
}

impl InitialRequest {
    fn send(self) -> bool {
        match self {
            Self::Hidden => false,
            Self::Show => super::daemon::send_show(),
            Self::ShowReverse => super::daemon::send_show_reverse(),
        }
    }

    fn show_on_start(self) -> bool {
        matches!(self, Self::Show)
    }
}

fn run_picker(initial_request: InitialRequest) -> CommandResult {
    if initial_request.send() {
        return CommandResult::success("");
    }

    let config = load_alt_tab_config();
    let (tx, rx) = mpsc::channel();

    if !super::daemon::start_listener(tx) {
        initial_request.send();
        return CommandResult::success("");
    }

    crate::preview_plane::prepare();
    let host = PickerHost {
        load_config: load_alt_tab_config,
        open_settings: open_settings_panel,
    };
    crate::picker::run::run_app(config, rx, initial_request.show_on_start(), host);
    super::daemon::cleanup();
    CommandResult::success("")
}
