use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use super::qol::key_of;
use super::{start, Level, Message, Relay};

const WAIT: Duration = Duration::from_secs(5);

struct Fake {
    seen: Sender<String>,
}

impl Relay for Fake {
    fn deliver(&self, message: &Message) -> bool {
        self.seen.send(message.title.clone()).unwrap();
        message.title.starts_with("shown")
    }
}

fn run(messages: Vec<Message>) -> (Vec<String>, Vec<Message>) {
    let (sender, receiver) = mpsc::channel();
    for message in messages {
        sender.send(message).unwrap();
    }
    drop(sender);
    let (seen, seen_rx) = mpsc::channel();
    let (left, left_rx) = mpsc::channel();
    start(receiver, Fake { seen }, move |message| {
        left.send(message).unwrap()
    });
    let mut unshown = Vec::new();
    loop {
        match left_rx.recv_timeout(WAIT) {
            Ok(message) => unshown.push(message),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => panic!("the relay never finished"),
        }
    }
    (seen_rx.try_iter().collect(), unshown)
}

#[test]
fn messages_are_delivered_in_order_and_only_unshown_problems_come_back() {
    let messages = vec![
        Message::info("shown a", ""),
        Message::info("lost b", ""),
        Message::warn("shown c", ""),
        Message::warn("lost d", "why"),
        Message::error("lost e", ""),
        Message::error("shown f", ""),
        Message::info("lost g", ""),
    ];
    let titles: Vec<String> = messages
        .iter()
        .map(|message| message.title.clone())
        .collect();

    let (seen, unshown) = run(messages);

    assert_eq!(seen, titles);
    assert_eq!(
        unshown,
        [Message::warn("lost d", "why"), Message::error("lost e", ""),]
    );
}

#[test]
fn a_line_joins_the_title_and_the_body_only_when_there_is_a_body() {
    let cases = [
        (Message::info("Copied the path", ""), "Copied the path"),
        (
            Message::warn("Nothing was undone", "Press ctrl+z again."),
            "Nothing was undone. Press ctrl+z again.",
        ),
    ];
    for (message, line) in cases {
        assert_eq!(message.line(), line);
    }
}

#[test]
fn a_problem_never_shares_a_card_with_a_success() {
    assert_ne!(key_of(Level::Info), key_of(Level::Warn));
    assert_eq!(key_of(Level::Warn), key_of(Level::Error));
}
