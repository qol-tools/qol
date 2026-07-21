use std::sync::mpsc;

pub(crate) enum Monitor {}

pub(crate) fn start(_window_title: String, _tx: mpsc::Sender<()>) -> Option<Monitor> {
    None
}
