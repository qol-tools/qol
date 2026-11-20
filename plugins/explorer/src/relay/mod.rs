mod qol;
#[cfg(test)]
mod tests;

use std::sync::mpsc::Receiver;
use std::thread;

pub use qol::QolNotifications;

use crate::PLUGIN_ID;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub body: String,
    pub level: Level,
}

impl Message {
    pub fn info(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(Level::Info, title, body)
    }

    pub fn warn(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(Level::Warn, title, body)
    }

    pub fn error(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(Level::Error, title, body)
    }

    fn new(level: Level, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            level,
        }
    }

    pub fn line(&self) -> String {
        if self.body.is_empty() {
            return self.title.clone();
        }
        format!("{}. {}", self.title, self.body)
    }
}

pub trait Relay: Send + 'static {
    fn deliver(&self, message: &Message) -> bool;
}

pub fn start(
    messages: Receiver<Message>,
    relay: impl Relay,
    unshown: impl Fn(Message) + Send + 'static,
) {
    let spawned = thread::Builder::new()
        .name("explorer-relay".into())
        .spawn(move || run(&messages, &relay, &unshown));
    if let Err(error) = spawned {
        log::warn!("[{PLUGIN_ID}] could not start the message relay: {error}");
    }
}

fn run(messages: &Receiver<Message>, relay: &impl Relay, unshown: &impl Fn(Message)) {
    for message in messages {
        if !relay.deliver(&message) && message.level != Level::Info {
            unshown(message);
        }
    }
}
