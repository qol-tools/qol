use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::cli::{
    CliChordBinder, CliChordBinding, CliChordFileError, CliChordRelease, CliTakenChord,
};
use crate::ChordCommand;

const FILE: &str = "keybindings.json";
const SCHEMA: &str = "https://www.schemastore.org/claude-code-keybindings.json";
const DOCS: &str = "https://code.claude.com/docs/en/keybindings";
const CONTEXT: &str = "Chat";
const CONTEXTS_THAT_HOLD_A_CHORD: [&str; 2] = [CONTEXT, "Global"];
const COMMAND_ACTION: &str = "command:";
const BETWEEN_KEYS: &str = " ";
const UNBOUND: &str = "no action";
const NOT_AN_ACTION: &str = "a value that is not an action";

pub(super) struct ClaudeKeybindings;

enum Held {
    Free,
    Ours,
    Other(String),
}

enum Stripped {
    Untouched,
    Kept,
    Emptied,
}

impl CliChordBinder for ClaudeKeybindings {
    fn file(&self, home: &Path) -> PathBuf {
        home.join(FILE)
    }

    fn bind(
        &self,
        current: Option<&str>,
        wanted: &[ChordCommand],
    ) -> Result<CliChordBinding, CliChordFileError> {
        let mut document = document(current)?;
        let mut binding = CliChordBinding::default();
        for command in wanted {
            match holder(&document, command) {
                Held::Ours => {}
                Held::Free => binding.added.push(command.clone()),
                Held::Other(holder) => binding.taken.push(CliTakenChord {
                    wanted: command.clone(),
                    holder,
                }),
            }
        }
        if binding.added.is_empty() {
            return Ok(binding);
        }
        add(&mut document, &binding.added)?;
        binding.text = Some(rendered(&document)?);
        Ok(binding)
    }

    fn release(
        &self,
        current: &str,
        bound: &[ChordCommand],
    ) -> Result<CliChordRelease, CliChordFileError> {
        let mut document = document(Some(current))?;
        let Some(blocks) = document.get_mut("bindings").and_then(Value::as_array_mut) else {
            return Ok(CliChordRelease::default());
        };
        let mut released = false;
        blocks.retain_mut(|block| match strip(block, bound) {
            Stripped::Untouched => true,
            Stripped::Kept => {
                released = true;
                true
            }
            Stripped::Emptied => {
                released = true;
                false
            }
        });
        let leaves_nothing = blocks.is_empty();
        let text = if released {
            Some(rendered(&document)?)
        } else {
            None
        };
        Ok(CliChordRelease {
            text,
            leaves_nothing,
        })
    }
}

fn document(current: Option<&str>) -> Result<Value, CliChordFileError> {
    let Some(text) = current.filter(|text| !text.trim().is_empty()) else {
        return Ok(json!({ "$schema": SCHEMA, "$docs": DOCS, "bindings": [] }));
    };
    let document = serde_json::from_str::<Value>(text)
        .map_err(|error| CliChordFileError::new(format!("{FILE} is not valid JSON: {error}")))?;
    if !document.is_object() {
        return Err(CliChordFileError::new(format!(
            "{FILE} does not hold a JSON object"
        )));
    }
    Ok(document)
}

fn rendered(document: &Value) -> Result<String, CliChordFileError> {
    serde_json::to_string_pretty(document)
        .map(|text| format!("{text}\n"))
        .map_err(|error| CliChordFileError::new(format!("{FILE} could not be written: {error}")))
}

fn action(command: &ChordCommand) -> String {
    format!("{COMMAND_ACTION}{}", command.command)
}

fn stroke(stroke: &str) -> String {
    let lower = stroke.to_ascii_lowercase();
    let mut parts = lower
        .split('+')
        .map(|part| match part {
            "control" => "ctrl",
            other => other,
        })
        .collect::<Vec<_>>();
    let key = parts.pop().unwrap_or_default();
    parts.sort_unstable();
    parts.dedup();
    parts.push(key);
    parts.join("+")
}

fn normalized(keys: &str) -> String {
    keys.split_whitespace()
        .map(stroke)
        .collect::<Vec<_>>()
        .join(BETWEEN_KEYS)
}

fn bound_in<'a>(document: &'a Value, command: &ChordCommand) -> Vec<&'a Value> {
    let chord = command.chord.spelled(BETWEEN_KEYS);
    document
        .get("bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| {
            block
                .get("context")
                .and_then(Value::as_str)
                .is_some_and(|context| CONTEXTS_THAT_HOLD_A_CHORD.contains(&context))
        })
        .filter_map(|block| block.get("bindings")?.as_object())
        .flat_map(|bindings| bindings.iter())
        .filter(|(keys, _)| normalized(keys) == chord)
        .map(|(_, bound)| bound)
        .collect()
}

fn holder(document: &Value, command: &ChordCommand) -> Held {
    let action = action(command);
    let bound = bound_in(document, command);
    let other = bound
        .iter()
        .find(|bound| bound.as_str() != Some(action.as_str()))
        .copied();
    match (other, bound.is_empty()) {
        (Some(Value::String(other)), _) => Held::Other(other.clone()),
        (Some(Value::Null), _) => Held::Other(UNBOUND.to_owned()),
        (Some(_), _) => Held::Other(NOT_AN_ACTION.to_owned()),
        (None, true) => Held::Free,
        (None, false) => Held::Ours,
    }
}

fn holds_chat_bindings(block: &Value) -> bool {
    block.get("context").and_then(Value::as_str) == Some(CONTEXT)
        && block.get("bindings").is_some_and(Value::is_object)
}

fn add(document: &mut Value, added: &[ChordCommand]) -> Result<(), CliChordFileError> {
    let not_a_list = || CliChordFileError::new(format!("`bindings` in {FILE} is not a list"));
    let root = document.as_object_mut().ok_or_else(not_a_list)?;
    let blocks = root
        .entry("bindings")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(not_a_list)?;
    if !blocks.iter().any(holds_chat_bindings) {
        blocks.push(json!({ "context": CONTEXT, "bindings": {} }));
    }
    let bindings = blocks
        .iter_mut()
        .find(|block| holds_chat_bindings(block))
        .and_then(|block| block.get_mut("bindings"))
        .and_then(Value::as_object_mut)
        .ok_or_else(not_a_list)?;
    for command in added {
        bindings.insert(
            command.chord.spelled(BETWEEN_KEYS),
            Value::String(action(command)),
        );
    }
    Ok(())
}

fn strip(block: &mut Value, bound: &[ChordCommand]) -> Stripped {
    if block.get("context").and_then(Value::as_str) != Some(CONTEXT) {
        return Stripped::Untouched;
    }
    let holds_nothing_else = block.as_object().is_some_and(|block| block.len() == 2);
    let Some(bindings) = block.get_mut("bindings").and_then(Value::as_object_mut) else {
        return Stripped::Untouched;
    };
    let before = bindings.len();
    bindings.retain(|keys, held| {
        !bound.iter().any(|command| {
            normalized(keys) == command.chord.spelled(BETWEEN_KEYS)
                && held.as_str() == Some(action(command).as_str())
        })
    });
    if bindings.len() == before {
        return Stripped::Untouched;
    }
    if bindings.is_empty() && holds_nothing_else {
        Stripped::Emptied
    } else {
        Stripped::Kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Chord;

    fn chords() -> Vec<ChordCommand> {
        vec![
            ChordCommand::new(Chord::ctrl("xx").unwrap(), "close-fork"),
            ChordCommand::new(Chord::ctrl("xc").unwrap(), "interrupt-fork"),
        ]
    }

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn a_missing_or_blank_file_gets_a_chat_block_with_every_chord() {
        for current in [None, Some(""), Some("  \n")] {
            let binding = ClaudeKeybindings.bind(current, &chords()).unwrap();
            assert_eq!(binding.added, chords(), "{current:?}");
            assert!(binding.taken.is_empty(), "{current:?}");
            assert_eq!(
                parsed(&binding.text.unwrap()),
                json!({
                    "$schema": SCHEMA,
                    "$docs": DOCS,
                    "bindings": [{ "context": "Chat", "bindings": {
                        "ctrl+x ctrl+x": "command:close-fork",
                        "ctrl+x ctrl+c": "command:interrupt-fork",
                    } }],
                }),
                "{current:?}"
            );
        }
    }

    #[test]
    fn missing_chords_join_the_chat_block_the_user_already_has() {
        let current = r#"{"bindings":[
            {"context":"Global","bindings":{"ctrl+k ctrl+t":"app:toggleTodos"}},
            {"context":"Chat","bindings":{"ctrl+e":"chat:externalEditor","Control+X  ctrl+x":"command:close-fork"}}
        ]}"#;
        let binding = ClaudeKeybindings.bind(Some(current), &chords()).unwrap();
        assert_eq!(binding.added, chords()[1..]);
        assert!(binding.taken.is_empty());
        assert_eq!(
            parsed(&binding.text.unwrap()),
            json!({ "bindings": [
                { "context": "Global", "bindings": { "ctrl+k ctrl+t": "app:toggleTodos" } },
                { "context": "Chat", "bindings": {
                    "ctrl+e": "chat:externalEditor",
                    "Control+X  ctrl+x": "command:close-fork",
                    "ctrl+x ctrl+c": "command:interrupt-fork",
                } },
            ] })
        );
    }

    #[test]
    fn a_file_that_binds_every_chord_is_left_alone() {
        let current = r#"{"bindings":[{"context":"Chat","bindings":{"ctrl+x ctrl+x":"command:close-fork","ctrl+x ctrl+c":"command:interrupt-fork"}}]}"#;
        assert_eq!(
            ClaudeKeybindings.bind(Some(current), &chords()).unwrap(),
            CliChordBinding::default()
        );
    }

    #[test]
    fn a_chord_the_user_gave_to_something_else_is_reported_and_kept() {
        let cases = [
            (r#""chat:stash""#, "Chat", "chat:stash"),
            (r#""app:redraw""#, "Global", "app:redraw"),
            ("null", "Chat", "no action"),
            ("7", "Chat", "a value that is not an action"),
        ];
        for (held, context, holder) in cases {
            let current = format!(
                r#"{{"bindings":[{{"context":"{context}","bindings":{{"ctrl+x ctrl+c":{held}}}}}]}}"#
            );
            let binding = ClaudeKeybindings.bind(Some(&current), &chords()).unwrap();
            assert_eq!(binding.added, chords()[..1], "{held}");
            assert_eq!(
                binding.taken,
                [CliTakenChord {
                    wanted: chords()[1].clone(),
                    holder: holder.to_owned(),
                }],
                "{held}"
            );
            let text = parsed(&binding.text.unwrap());
            let kept = text["bindings"][0]["bindings"]["ctrl+x ctrl+c"].clone();
            assert_eq!(kept, parsed(held), "{held}");
        }
    }

    #[test]
    fn a_file_that_cannot_be_read_as_bindings_is_refused() {
        for current in ["{", "[]", r#"{"bindings":{}}"#] {
            assert!(
                ClaudeKeybindings.bind(Some(current), &chords()).is_err(),
                "{current}"
            );
        }
    }

    #[test]
    fn release_takes_out_only_what_still_holds_the_bound_command() {
        let current = r#"{"bindings":[
            {"context":"Global","bindings":{"ctrl+x ctrl+c":"command:interrupt-fork"}},
            {"context":"Chat","bindings":{"ctrl+e":"chat:externalEditor","ctrl+x ctrl+x":"chat:stash","ctrl+x ctrl+c":"command:interrupt-fork"}}
        ]}"#;
        let release = ClaudeKeybindings.release(current, &chords()).unwrap();
        assert!(!release.leaves_nothing);
        assert_eq!(
            parsed(&release.text.unwrap()),
            json!({ "bindings": [
                { "context": "Global", "bindings": { "ctrl+x ctrl+c": "command:interrupt-fork" } },
                { "context": "Chat", "bindings": { "ctrl+e": "chat:externalEditor", "ctrl+x ctrl+x": "chat:stash" } },
            ] })
        );
    }

    #[test]
    fn release_drops_the_block_it_emptied_and_says_when_nothing_is_left() {
        let bound = ClaudeKeybindings.bind(None, &chords()).unwrap();
        let release = ClaudeKeybindings
            .release(&bound.text.unwrap(), &chords())
            .unwrap();
        assert!(release.leaves_nothing);
        assert_eq!(
            parsed(&release.text.unwrap()),
            json!({ "$schema": SCHEMA, "$docs": DOCS, "bindings": [] })
        );
    }

    #[test]
    fn release_changes_nothing_when_the_user_took_the_chords_out() {
        let current =
            r#"{"bindings":[{"context":"Chat","bindings":{"ctrl+e":"chat:externalEditor"}}]}"#;
        assert_eq!(
            ClaudeKeybindings.release(current, &chords()).unwrap(),
            CliChordRelease {
                text: None,
                leaves_nothing: false,
            }
        );
    }
}
