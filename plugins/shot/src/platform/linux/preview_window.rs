use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use futures::channel::oneshot;
use qol_gpui::popup_window::{configure_popup_window, hide_invisible};

static PIN_TRANSITIONS: LazyLock<Mutex<HashMap<String, oneshot::Sender<bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn prepare_preview_window(title: &str) -> bool {
    let configured = configure_popup_window(title);
    if !qol_gpui::popup_window::set_override_redirect_by_title(title) {
        return false;
    }
    if !configured {
        configure_popup_window(title);
    }
    hide_invisible(title);
    true
}

pub(crate) fn register_pin_transition(title: &str) -> Option<oneshot::Receiver<bool>> {
    let (sender, receiver) = oneshot::channel();
    PIN_TRANSITIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(title.to_owned(), sender);
    Some(receiver)
}

pub(crate) fn complete_pin_transition(title: &str, succeeded: bool) {
    let sender = PIN_TRANSITIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(title);
    if let Some(sender) = sender {
        let _ = sender.send(succeeded);
    }
}

#[cfg(test)]
mod tests {
    use super::{complete_pin_transition, register_pin_transition};

    #[test]
    fn pin_transition_completion_reaches_the_preview_once() {
        let source = "pin-transition-test-preview";
        let failed = register_pin_transition(source).unwrap();
        complete_pin_transition(source, false);
        assert!(!futures::executor::block_on(failed).unwrap());
        complete_pin_transition(source, true);

        let succeeded = register_pin_transition(source).unwrap();
        complete_pin_transition(source, true);
        assert!(futures::executor::block_on(succeeded).unwrap());
    }
}
