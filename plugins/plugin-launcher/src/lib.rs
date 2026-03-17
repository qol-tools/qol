pub mod daemon;
pub mod discovery;
pub mod launch;
pub mod ui;

pub use qol_frecency as frecency;
pub use qol_plugin_api::monitor;
pub use qol_plugin_api::window::open_window_with_focus;
pub use qol_search::{fuzzy_match, FuzzyMatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchAction {
    Open,
    Terminal,
    OpenFolder,
    CopyPath,
}

pub fn action_for_modifiers(ctrl: bool, shift: bool, alt: bool) -> LaunchAction {
    if ctrl {
        LaunchAction::Terminal
    } else if shift {
        LaunchAction::OpenFolder
    } else if alt {
        LaunchAction::CopyPath
    } else {
        LaunchAction::Open
    }
}

pub fn action_hint(ctrl: bool, shift: bool, alt: bool) -> Option<LaunchAction> {
    if ctrl || shift || alt {
        Some(action_for_modifiers(ctrl, shift, alt))
    } else {
        None
    }
}

pub fn action_label(action: LaunchAction) -> &'static str {
    match action {
        LaunchAction::Open => "Open",
        LaunchAction::Terminal => "Open in Terminal",
        LaunchAction::OpenFolder => "Open Folder",
        LaunchAction::CopyPath => "Copy Path",
    }
}
