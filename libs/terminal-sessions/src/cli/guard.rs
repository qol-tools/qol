use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::{
    DeliveryMode, SessionBinding, SessionFacts, SessionInventory, TerminalError,
    TerminalSessionService, TextInput,
};

use super::{composer_draft, composer_screen, CliComposer, CliSessionInterpreter};

pub const QUIET: Duration = Duration::from_secs(3);
const RETRY: Duration = Duration::from_millis(500);
const ECHO_WINDOW: Duration = Duration::from_millis(1500);
const ECHO_POLL: Duration = Duration::from_millis(100);
const SUBMIT_KEY: &str = "enter";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hold {
    Draft,
    Unreadable,
    Typing,
}

impl std::fmt::Display for Hold {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Draft => "its input box holds a draft",
            Self::Unreadable => "its input box is not on screen",
            Self::Typing => "its input box was in use",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Typed {
    Submitted,
    Inserted,
    Unsubmitted,
    Held(Hold),
}

impl Typed {
    pub fn landed(self) -> bool {
        matches!(self, Self::Submitted | Self::Inserted)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Reading {
    Modeled(CliComposer),
    Loose(CliComposer),
    Blind,
}

impl Reading {
    fn composer(&self) -> Option<&CliComposer> {
        match self {
            Self::Modeled(composer) | Self::Loose(composer) => Some(composer),
            Self::Blind => None,
        }
    }
}

struct Sighting {
    reading: Reading,
    since: Instant,
}

#[derive(Default)]
pub struct ComposerGuard {
    sightings: HashMap<SessionBinding, Sighting>,
    typed: HashMap<SessionBinding, String>,
}

#[derive(Clone, Copy)]
pub struct Typing<'a> {
    pub binding: &'a SessionBinding,
    pub text: &'a str,
    pub mode: DeliveryMode,
}

impl<'a> Typing<'a> {
    pub fn submit(binding: &'a SessionBinding, text: &'a str) -> Self {
        Self {
            binding,
            text,
            mode: DeliveryMode::Submit,
        }
    }
}

impl ComposerGuard {
    pub fn type_text(
        &mut self,
        terminals: &TerminalSessionService,
        interpreter: &CliSessionInterpreter,
        typing: Typing<'_>,
    ) -> Result<Typed, TerminalError> {
        let Typing {
            binding,
            text,
            mode,
        } = typing;
        let facts = facts_for(terminals, binding)?;
        let reading = read(terminals, interpreter, &facts, binding)?;
        if let Some(composer) = reading.composer() {
            if self.typed.get(binding).is_some_and(|typed| typed == text)
                && composer.holds_only(text)
            {
                return self.submit(terminals, binding, mode);
            }
        }
        let now = Instant::now();
        let since = self.sighted(binding, &reading, now);
        if let Err(hold) = clearance(&reading, facts.focused, now.duration_since(since)) {
            return Ok(Typed::Held(hold));
        }
        self.sightings.remove(binding);
        if !matches!(reading, Reading::Modeled(_)) {
            terminals.send_text(binding, text, mode)?;
            return Ok(landed(mode));
        }
        terminals.send_text(binding, text, DeliveryMode::Insert)?;
        self.typed.insert(binding.clone(), text.to_owned());
        if !echoed(terminals, interpreter, &facts, binding, text) {
            return Ok(Typed::Unsubmitted);
        }
        self.submit(terminals, binding, mode)
    }

    pub fn type_text_within(
        &mut self,
        terminals: &TerminalSessionService,
        interpreter: &CliSessionInterpreter,
        typing: Typing<'_>,
        wait: Duration,
        sleep: &mut dyn FnMut(Duration),
    ) -> Result<Typed, TerminalError> {
        let mut waited = Duration::ZERO;
        loop {
            let typed = self.type_text(terminals, interpreter, typing)?;
            if !matches!(typed, Typed::Held(_)) || waited >= wait {
                return Ok(typed);
            }
            sleep(RETRY);
            waited += RETRY;
        }
    }

    fn sighted(&mut self, binding: &SessionBinding, reading: &Reading, now: Instant) -> Instant {
        match self.sightings.get(binding) {
            Some(sighting) if sighting.reading == *reading => sighting.since,
            _ => {
                self.sightings.insert(
                    binding.clone(),
                    Sighting {
                        reading: reading.clone(),
                        since: now,
                    },
                );
                now
            }
        }
    }

    fn submit(
        &mut self,
        terminals: &TerminalSessionService,
        binding: &SessionBinding,
        mode: DeliveryMode,
    ) -> Result<Typed, TerminalError> {
        if mode == DeliveryMode::Submit {
            terminals.send_key(binding, SUBMIT_KEY)?;
        }
        self.typed.remove(binding);
        Ok(landed(mode))
    }
}

pub fn read_composer(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    binding: &SessionBinding,
) -> Result<CliComposer, TerminalError> {
    let facts = facts_for(terminals, binding)?;
    let screen = composer_screen(terminals, binding)?;
    Ok(interpreter
        .composer(&facts, &screen)
        .unwrap_or_else(|| loose(&screen)))
}

fn clearance(reading: &Reading, focused: bool, unchanged_for: Duration) -> Result<(), Hold> {
    match reading.composer() {
        None => Ok(()),
        Some(CliComposer::Draft { .. }) => Err(Hold::Draft),
        Some(CliComposer::Unreadable) => Err(Hold::Unreadable),
        Some(CliComposer::Empty) if focused && unchanged_for < QUIET => Err(Hold::Typing),
        Some(CliComposer::Empty) => Ok(()),
    }
}

fn facts_for(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
) -> Result<SessionFacts, TerminalError> {
    terminals
        .discover()?
        .into_iter()
        .find(|facts| facts.binding().as_ref() == Ok(binding))
        .ok_or_else(|| TerminalError::TargetMissing(binding.clone()))
}

fn read(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    facts: &SessionFacts,
    binding: &SessionBinding,
) -> Result<Reading, TerminalError> {
    match composer_screen(terminals, binding) {
        Ok(screen) => Ok(match interpreter.composer(facts, &screen) {
            Some(composer) => Reading::Modeled(composer),
            None => Reading::Loose(loose(&screen)),
        }),
        Err(TerminalError::Unsupported { .. }) => Ok(Reading::Blind),
        Err(error) => Err(error),
    }
}

fn loose(screen: &str) -> CliComposer {
    match composer_draft(screen) {
        Some(text) => CliComposer::Draft { text, pastes: 0 },
        None => CliComposer::Empty,
    }
}

fn echoed(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    facts: &SessionFacts,
    binding: &SessionBinding,
    text: &str,
) -> bool {
    let started = Instant::now();
    loop {
        if let Ok(Reading::Modeled(composer)) = read(terminals, interpreter, facts, binding) {
            if composer.holds_only(text) {
                return true;
            }
        }
        if started.elapsed() >= ECHO_WINDOW {
            return false;
        }
        std::thread::sleep(ECHO_POLL);
    }
}

fn landed(mode: DeliveryMode) -> Typed {
    match mode {
        DeliveryMode::Submit => Typed::Submitted,
        DeliveryMode::Insert => Typed::Inserted,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::{clearance, ComposerGuard, Hold, Reading, Typed, Typing, QUIET};
    use crate::cli::{CliComposer, CliSessionInterpreter};
    use crate::{
        BackendId, DeliveryMode, ScreenReader, SessionBinding, SessionCapabilities, SessionFacts,
        SessionFocus, SessionId, SessionInventory, TerminalBackend, TerminalError,
        TerminalSessionService, TerminalSnapshot, TextInput,
    };

    struct ClaudePane {
        facts: SessionFacts,
        screens: Mutex<VecDeque<String>>,
        keys: Mutex<Vec<String>>,
    }

    impl ClaudePane {
        fn showing(inputs: &[&str]) -> Arc<Self> {
            let rule = "\u{2500}".repeat(40);
            Arc::new(Self {
                facts: SessionFacts {
                    id: SessionId::new(BackendId::new("fake").unwrap(), "1").unwrap(),
                    root_pid: 7,
                    cwd: "/work".to_owned(),
                    title: "claude".to_owned(),
                    at_prompt: true,
                    focused: false,
                    reported_cmd: None,
                    foreground_basenames: vec!["claude".to_owned()],
                    foreground_pids: Vec::new(),
                    capabilities: SessionCapabilities::ALL,
                    spawn_identity: None,
                },
                screens: Mutex::new(
                    inputs
                        .iter()
                        .map(|input| format!("{rule}\n\u{276F} {input}\n{rule}\n  /work"))
                        .collect(),
                ),
                keys: Mutex::new(Vec::new()),
            })
        }
    }

    impl SessionInventory for ClaudePane {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            Ok(vec![self.facts.clone()])
        }
    }

    impl ScreenReader for ClaudePane {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            let mut screens = self.screens.lock().unwrap();
            if screens.len() > 1 {
                return Ok(screens.pop_front().unwrap());
            }
            Ok(screens.front().cloned().unwrap_or_default())
        }

        fn read_screen_styled(&self, target: &SessionBinding) -> Result<String, TerminalError> {
            self.read_screen(target)
        }
    }

    impl SessionFocus for ClaudePane {
        fn focus(&self, _target: &SessionBinding) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TextInput for ClaudePane {
        fn send_text(
            &self,
            _target: &SessionBinding,
            _text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            Ok(())
        }

        fn send_key(&self, _target: &SessionBinding, key: &str) -> Result<(), TerminalError> {
            self.keys.lock().unwrap().push(key.to_owned());
            Ok(())
        }
    }

    impl TerminalBackend for ClaudePane {
        fn read_screen_from_snapshot(
            &self,
            _snapshot: &TerminalSnapshot,
            target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            self.read_screen(target)
        }

        fn id(&self) -> &BackendId {
            self.facts.id.backend()
        }
    }

    #[test]
    fn a_submitted_text_never_submits_a_later_draft_that_ends_like_it() {
        let knock = "you have a message";
        let pane = ClaudePane::showing(&["", knock, "message"]);
        let terminals =
            TerminalSessionService::from_backends([Arc::clone(&pane) as Arc<dyn TerminalBackend>])
                .unwrap();
        let interpreter = CliSessionInterpreter::system();
        let binding = pane.facts.binding().unwrap();
        let mut guard = ComposerGuard::default();
        let typing = Typing::submit(&binding, knock);

        assert_eq!(
            guard.type_text(&terminals, &interpreter, typing).unwrap(),
            Typed::Submitted
        );
        assert_eq!(
            guard.type_text(&terminals, &interpreter, typing).unwrap(),
            Typed::Held(Hold::Draft)
        );
        assert_eq!(*pane.keys.lock().unwrap(), ["enter"]);
    }

    #[test]
    fn a_draft_or_an_unreadable_box_always_holds_the_text() {
        let draft = CliComposer::Draft {
            text: "half a messa".to_owned(),
            pastes: 0,
        };
        for focused in [false, true] {
            for unchanged_for in [Duration::ZERO, QUIET * 10] {
                for reading in [
                    Reading::Modeled(draft.clone()),
                    Reading::Loose(draft.clone()),
                ] {
                    assert_eq!(
                        clearance(&reading, focused, unchanged_for),
                        Err(Hold::Draft)
                    );
                }
                assert_eq!(
                    clearance(
                        &Reading::Modeled(CliComposer::Unreadable),
                        focused,
                        unchanged_for
                    ),
                    Err(Hold::Unreadable)
                );
            }
        }
    }

    #[test]
    fn a_focused_box_the_user_just_emptied_waits_until_it_stays_quiet() {
        let empty = Reading::Modeled(CliComposer::Empty);
        let cases = [
            (true, Duration::ZERO, Err(Hold::Typing)),
            (true, QUIET - Duration::from_millis(1), Err(Hold::Typing)),
            (true, QUIET, Ok(())),
            (false, Duration::ZERO, Ok(())),
        ];
        for (focused, unchanged_for, expected) in cases {
            assert_eq!(
                clearance(&empty, focused, unchanged_for),
                expected,
                "focused={focused} unchanged_for={unchanged_for:?}"
            );
        }
        assert_eq!(clearance(&Reading::Blind, true, Duration::ZERO), Ok(()));
    }

    #[test]
    fn the_quiet_clock_restarts_whenever_the_box_changes() {
        let binding = SessionBinding::new(
            SessionId::new(BackendId::new("kitty").unwrap(), "k1:7").unwrap(),
            42,
        )
        .unwrap();
        let empty = Reading::Modeled(CliComposer::Empty);
        let typed = Reading::Modeled(CliComposer::Draft {
            text: "h".to_owned(),
            pastes: 0,
        });
        let start = Instant::now();
        let later = |seconds| start + Duration::from_secs(seconds);
        let mut guard = ComposerGuard::default();
        assert_eq!(guard.sighted(&binding, &empty, start), start);
        assert_eq!(guard.sighted(&binding, &empty, later(2)), start);
        assert_eq!(guard.sighted(&binding, &typed, later(3)), later(3));
        assert_eq!(guard.sighted(&binding, &empty, later(4)), later(4));
        assert_eq!(guard.sighted(&binding, &empty, later(9)), later(4));
    }
}
