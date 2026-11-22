use std::io::{IsTerminal, Read, Write};

use anyhow::{bail, Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal;
use zeroize::Zeroizing;

const CAPACITY: usize = 64 * 1024;

pub(super) fn read_secret(name: &str) -> Result<Zeroizing<String>> {
    let raw = if std::io::stdin().is_terminal() {
        prompt(name)?
    } else {
        read_stdin(std::io::stdin().lock())?
    };
    let secret = trimmed(&raw);
    if secret.is_empty() {
        bail!("no secret was given for `{name}`; nothing was stored");
    }
    Ok(secret)
}

fn trimmed(raw: &str) -> Zeroizing<String> {
    let mut secret = Zeroizing::new(String::with_capacity(raw.len()));
    secret.push_str(raw.trim_matches(|character: char| character.is_ascii_whitespace()));
    secret
}

fn read_stdin(stdin: impl Read) -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::<u8>::with_capacity(CAPACITY + 64));
    stdin
        .take(CAPACITY as u64 + 1)
        .read_to_end(&mut bytes)
        .context("failed to read the secret from stdin")?;
    if bytes.len() > CAPACITY {
        bail!("stdin holds more than {CAPACITY} bytes, which is no secret qol stores");
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("stdin is not UTF-8"))?;
    let mut secret = Zeroizing::new(String::with_capacity(text.len()));
    secret.push_str(text);
    Ok(secret)
}

fn prompt(name: &str) -> Result<Zeroizing<String>> {
    let mut stderr = std::io::stderr();
    write!(
        stderr,
        "Secret for {name} (hidden, Enter to store, Esc to cancel): "
    )
    .and_then(|()| stderr.flush())
    .context("failed to write the prompt")?;
    let raw_mode = RawMode::enter()?;
    let mut secret = Zeroizing::new(String::with_capacity(CAPACITY));
    let end = loop {
        let end = match event::read().context("failed to read the terminal")? {
            Event::Key(key) => apply_key(&mut secret, key),
            _ => None,
        };
        if let Some(end) = end {
            break end;
        }
    };
    drop(raw_mode);
    let _ = writeln!(stderr);
    match end {
        End::Done => Ok(secret),
        End::Cancel => bail!("cancelled; nothing was stored under `{name}`"),
        End::TooLong => bail!("the secret for `{name}` is longer than {CAPACITY} bytes"),
    }
}

struct RawMode;

impl RawMode {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to hide the typed secret on this terminal")?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Done,
    Cancel,
    TooLong,
}

fn apply_key(secret: &mut Zeroizing<String>, key: KeyEvent) -> Option<End> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let shortcut =
        key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Enter => Some(End::Done),
        KeyCode::Esc => Some(End::Cancel),
        KeyCode::Char('c' | 'd') if shortcut => Some(End::Cancel),
        KeyCode::Char('u') if shortcut => {
            secret.clear();
            None
        }
        KeyCode::Backspace => {
            secret.pop();
            None
        }
        KeyCode::Char(character) if !shortcut => {
            apply_text(secret, character.encode_utf8(&mut [0; 4]))
        }
        _ => None,
    }
}

fn apply_text(secret: &mut Zeroizing<String>, text: &str) -> Option<End> {
    if secret.len() + text.len() > CAPACITY {
        return Some(End::TooLong);
    }
    secret.push_str(text);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn typed(keys: &[KeyEvent]) -> (String, Option<End>) {
        let mut secret = Zeroizing::new(String::with_capacity(CAPACITY));
        for key in keys {
            if let Some(end) = apply_key(&mut secret, *key) {
                return (secret.to_string(), Some(end));
            }
        }
        (secret.to_string(), None)
    }

    fn chars(text: &str) -> Vec<KeyEvent> {
        text.chars()
            .map(|character| key(KeyCode::Char(character), KeyModifiers::NONE))
            .collect()
    }

    #[test]
    fn typing_then_enter_gives_the_typed_secret() {
        let mut keys = chars("sk-Ab C");
        keys.push(key(KeyCode::Char('D'), KeyModifiers::SHIFT));
        keys.push(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(typed(&keys), ("sk-Ab CD".to_owned(), Some(End::Done)));
    }

    #[test]
    fn backspace_and_ctrl_u_edit_the_secret() {
        let mut keys = chars("abc");
        keys.push(key(KeyCode::Backspace, KeyModifiers::NONE));
        keys.extend(chars("x"));
        keys.push(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(typed(&keys), ("abx".to_owned(), Some(End::Done)));
        let mut keys = chars("abc");
        keys.push(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        keys.extend(chars("z"));
        keys.push(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(typed(&keys), ("z".to_owned(), Some(End::Done)));
    }

    #[test]
    fn esc_ctrl_c_and_ctrl_d_cancel() {
        for cancel in [
            key(KeyCode::Esc, KeyModifiers::NONE),
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            key(KeyCode::Char('d'), KeyModifiers::CONTROL),
        ] {
            let mut keys = chars("abc");
            keys.push(cancel);
            assert_eq!(typed(&keys).1, Some(End::Cancel));
        }
    }

    #[test]
    fn key_releases_and_other_keys_type_nothing() {
        let mut release = key(KeyCode::Char('x'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        let keys = [
            release,
            key(KeyCode::Left, KeyModifiers::NONE),
            key(KeyCode::Char('v'), KeyModifiers::CONTROL),
            key(KeyCode::Tab, KeyModifiers::NONE),
            key(KeyCode::Enter, KeyModifiers::NONE),
        ];
        assert_eq!(typed(&keys), (String::new(), Some(End::Done)));
    }

    #[test]
    fn altgr_characters_are_typed_like_any_other() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        let keys = [
            key(KeyCode::Char('@'), altgr),
            key(KeyCode::Enter, KeyModifiers::NONE),
        ];
        assert_eq!(typed(&keys), ("@".to_owned(), Some(End::Done)));
    }

    #[test]
    fn input_past_the_buffer_stops_the_prompt() {
        let mut secret = Zeroizing::new(String::with_capacity(CAPACITY));
        assert_eq!(apply_text(&mut secret, &"a".repeat(CAPACITY)), None);
        assert_eq!(apply_text(&mut secret, "b"), Some(End::TooLong));
    }

    #[test]
    fn piped_input_loses_its_line_break_and_surrounding_spaces() {
        let raw = read_stdin(&b"  sk-ant-oat01-AbC\r\n"[..]).unwrap();
        assert_eq!(trimmed(&raw).as_str(), "sk-ant-oat01-AbC");
    }

    #[test]
    fn oversized_or_non_utf8_stdin_is_refused_without_its_content() {
        let big = vec![b'p'; CAPACITY + 1];
        let error = read_stdin(&big[..]).unwrap_err().to_string();
        assert!(!error.contains("pppp"), "{error}");
        let error = read_stdin(&[b'p', 0xff][..]).unwrap_err().to_string();
        assert_eq!(error, "stdin is not UTF-8");
    }
}
