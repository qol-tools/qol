use std::sync::mpsc;

use qol_headless::CommandResult;

use crate::config::{load_alt_tab_config, PLUGIN_ID};

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

pub(crate) fn open_settings_page() {
    if let Err(error) = qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID) {
        eprintln!("Failed to open settings page: {error}");
    }
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
    crate::picker::run::run_app(config, rx, initial_request.show_on_start());
    super::daemon::cleanup();
    CommandResult::success("")
}
