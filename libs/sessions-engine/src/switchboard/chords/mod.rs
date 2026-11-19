mod journal;

use std::ffi::OsString;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use qol_terminal_sessions::cli::{CliChordBinder, CliSessionInterpreter, CliToolId};
use qol_terminal_sessions::{Chord, ChordCommand, TerminalSessionService};

use self::journal::{Host, Record};

pub(super) const RELEASE: &str = "release-chords";
const BETWEEN_KEYS: &str = " ";
const WANTED: [(&str, &str); 2] = [("xx", "close-fork"), ("xc", "interrupt-fork")];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Bound {
    launch: String,
    chords: Vec<ChordCommand>,
}

impl Bound {
    pub(super) fn handoff(&self) -> String {
        let chords = self
            .chords
            .iter()
            .map(|bound| {
                serde_json::json!({
                    "keys": bound.chord.spelled(BETWEEN_KEYS),
                    "command": bound.command,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({ "launch": self.launch, "chords": chords }).to_string()
    }
}

struct Binding {
    bound: Vec<ChordCommand>,
    warnings: Vec<String>,
}

fn wanted() -> Vec<ChordCommand> {
    WANTED
        .iter()
        .map(|(letters, command)| {
            let chord = Chord::ctrl(letters).expect("the switchboard's chords are ctrl letters");
            ChordCommand::new(chord, command)
        })
        .collect()
}

fn spelled(chords: &[ChordCommand]) -> String {
    chords
        .iter()
        .map(|bound| bound.chord.spelled(BETWEEN_KEYS))
        .collect::<Vec<_>>()
        .join(", ")
}

fn launch_id() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("{:x}-{since_epoch:x}", std::process::id())
}

fn bind(
    host: &Host,
    binder: &dyn CliChordBinder,
    tool: &CliToolId,
    home: &Path,
    launch: &str,
) -> Result<Binding> {
    let wanted = wanted();
    let _lock = host.lock()?;
    let mut warnings = host.take_over(launch);
    let file = binder.file(home);
    let current = journal::read(&file)?;
    let planned = binder
        .bind(current.as_deref(), &wanted)
        .with_context(|| format!("{} is left as it is", file.display()))?;
    warnings.extend(planned.taken.iter().map(|taken| {
        format!(
            "{} gives {} to {}, so it does not run /{}",
            file.display(),
            taken.wanted.chord.spelled(BETWEEN_KEYS),
            taken.holder,
            taken.wanted.command
        )
    }));
    let held = |unwritten: &[ChordCommand]| {
        wanted
            .iter()
            .filter(|chord| !planned.taken.iter().any(|taken| taken.wanted == **chord))
            .filter(|chord| !unwritten.contains(chord))
            .cloned()
            .collect::<Vec<_>>()
    };
    let Some(text) = &planned.text else {
        return Ok(Binding {
            bound: held(&[]),
            warnings,
        });
    };
    let recorded = planned.added.iter().try_for_each(|added| {
        host.claim(Record::new(launch, tool, &file, current.is_none(), added))
    });
    let written = match recorded {
        Ok(()) => journal::write(&file, text),
        Err(error) if host.keeps_what_it_binds() => {
            log::warn!(
                "the key chords added to {} were not recorded: {error:#}",
                file.display()
            );
            journal::write(&file, text)
        }
        Err(error) => Err(error.context("what qol adds could not be recorded to take out again")),
    };
    let Err(error) = written else {
        return Ok(Binding {
            bound: held(&[]),
            warnings,
        });
    };
    warnings.push(format!(
        "{} is unchanged, so {} are not set up: {error:#}",
        file.display(),
        spelled(&planned.added)
    ));
    warnings.extend(host.hand_back(launch));
    Ok(Binding {
        bound: held(&planned.added),
        warnings,
    })
}

fn bind_on_this_host(
    interpreter: &CliSessionInterpreter,
    tool: &str,
    home: &Path,
    launch: &str,
) -> Result<Binding> {
    let tool = CliToolId::new(tool.to_owned())
        .map_err(|error| anyhow!("invalid tool `{tool}`: {error}"))?;
    let binder = interpreter
        .chord_binder_for(&tool)
        .with_context(|| format!("{tool} has no key chords file qol can write"))?;
    bind(&Host::system()?, binder, &tool, home, launch)
}

pub(super) fn bind_for(interpreter: &CliSessionInterpreter, tool: &str, home: &Path) -> Bound {
    let launch = launch_id();
    let chords = match bind_on_this_host(interpreter, tool, home, &launch) {
        Ok(binding) => {
            for warning in binding.warnings {
                crate::cli::warn(warning);
            }
            binding.bound
        }
        Err(error) => {
            crate::cli::warn(format!(
                "the switchboard's key chords are not set up: {error:#}"
            ));
            Vec::new()
        }
    };
    Bound { launch, chords }
}

fn pass_through(terminals: &TerminalSessionService, chords: &[ChordCommand]) -> Result<()> {
    let snapshot = terminals.snapshot().context("session discovery failed")?;
    let held = snapshot
        .sessions()
        .iter()
        .find(|facts| super::holds_switchboard_key(facts))
        .context("the switchboard's window is missing from discovery")?;
    let binding = held
        .binding()
        .context("the switchboard's window cannot bind to a stable token")?;
    let keys = chords
        .iter()
        .map(|bound| bound.chord.clone())
        .collect::<Vec<_>>();
    terminals.pass_chords(&binding, &keys)?;
    Ok(())
}

pub(super) fn pass(terminals: &TerminalSessionService, bound: &Bound) {
    if bound.chords.is_empty() {
        return;
    }
    if let Err(error) = pass_through(terminals, &bound.chords) {
        crate::cli::warn(format!(
            "{} reach the switchboard only where the terminal does not keep them for itself: {error:#}",
            spelled(&bound.chords)
        ));
    }
}

fn settle_on_this_host(settle: impl Fn(&Host) -> Vec<String>) {
    let settled = Host::system().and_then(|host| {
        let _lock = host.lock()?;
        Ok(settle(&host))
    });
    match settled {
        Ok(warnings) => {
            for warning in warnings {
                crate::cli::warn(warning);
            }
        }
        Err(error) => crate::cli::warn(format!(
            "the switchboard's key chords stay bound: {error:#}"
        )),
    }
}

pub(super) struct ReleaseAtExit(String);

impl ReleaseAtExit {
    pub(super) fn of(bound: &Bound) -> Self {
        Self(bound.launch.clone())
    }
}

impl Drop for ReleaseAtExit {
    fn drop(&mut self) {
        release(&self.0);
    }
}

pub(super) fn release(launch: &str) {
    settle_on_this_host(|host| host.release(launch));
}

pub(super) fn hand_back(bound: &Bound) {
    settle_on_this_host(|host| host.hand_back(&bound.launch));
}

pub(super) fn run_release(args: &[OsString]) -> Result<()> {
    let launch = args
        .first()
        .and_then(|arg| arg.to_str())
        .ok_or_else(|| anyhow!("{RELEASE} takes the launch id"))?;
    release(launch);
    Ok(())
}

#[cfg(test)]
mod tests {
    use qol_host_session::Residency;
    use serde_json::{json, Value};

    use super::*;

    const TOOL: &str = "claude";
    const OURS: &str = r#"{"bindings":[{"context":"Chat","bindings":{"ctrl+x ctrl+x":"command:close-fork","ctrl+x ctrl+c":"command:interrupt-fork"}}]}"#;

    struct Rig {
        dir: tempfile::TempDir,
        interpreter: CliSessionInterpreter,
        tool: CliToolId,
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("home")).unwrap();
            Self {
                dir,
                interpreter: CliSessionInterpreter::system(),
                tool: CliToolId::new(TOOL.to_owned()).unwrap(),
            }
        }

        fn host(&self, residency: Residency) -> Host {
            Host::new(self.dir.path().join("ledger"), residency)
        }

        fn file(&self) -> std::path::PathBuf {
            self.dir.path().join("home").join("keybindings.json")
        }

        fn bind(&self, host: &Host, launch: &str) -> Binding {
            let binder = self.interpreter.chord_binder_for(&self.tool).unwrap();
            bind(
                host,
                binder,
                &self.tool,
                &self.dir.path().join("home"),
                launch,
            )
            .unwrap()
        }

        fn chat(&self) -> Value {
            let text = std::fs::read_to_string(self.file()).unwrap();
            serde_json::from_str::<Value>(&text).unwrap()["bindings"][0]["bindings"].clone()
        }
    }

    #[test]
    fn a_portable_host_gets_both_chords_and_loses_them_when_that_launch_is_released() {
        let rig = Rig::new();
        let host = rig.host(Residency::Portable);

        let binding = rig.bind(&host, "first");

        assert_eq!(binding.bound, wanted());
        assert!(binding.warnings.is_empty(), "{:?}", binding.warnings);
        assert_eq!(
            rig.chat(),
            json!({
                "ctrl+x ctrl+x": "command:close-fork",
                "ctrl+x ctrl+c": "command:interrupt-fork",
            })
        );

        assert!(host.release("another").is_empty());
        assert!(rig.file().exists());

        assert!(host.release("first").is_empty());
        assert!(!rig.file().exists());
        assert!(host.release("first").is_empty());
    }

    #[test]
    fn a_resident_host_keeps_the_chords_after_a_release() {
        let rig = Rig::new();
        let host = rig.host(Residency::Resident);

        assert_eq!(rig.bind(&host, "first").bound, wanted());
        assert!(host.release("first").is_empty());

        assert_eq!(rig.chat().as_object().unwrap().len(), 2);
        assert_eq!(rig.bind(&host, "second").bound, wanted());
    }

    #[test]
    fn chords_the_user_bound_are_used_and_never_taken_out() {
        let rig = Rig::new();
        let host = rig.host(Residency::Portable);
        std::fs::write(rig.file(), OURS).unwrap();

        let binding = rig.bind(&host, "first");

        assert_eq!(binding.bound, wanted());
        assert!(host.release("first").is_empty());
        assert_eq!(std::fs::read_to_string(rig.file()).unwrap(), OURS);
    }

    #[test]
    fn a_chord_the_user_gave_away_is_reported_and_only_the_other_is_bound() {
        let rig = Rig::new();
        let host = rig.host(Residency::Portable);
        std::fs::write(
            rig.file(),
            r#"{"bindings":[{"context":"Chat","bindings":{"ctrl+e":"chat:externalEditor","ctrl+x ctrl+c":"chat:stash"}}]}"#,
        )
        .unwrap();

        let binding = rig.bind(&host, "first");

        assert_eq!(binding.bound, wanted()[..1]);
        assert_eq!(binding.warnings.len(), 1);
        assert!(
            binding.warnings[0].ends_with(
                "keybindings.json gives ctrl+x ctrl+c to chat:stash, so it does not run /interrupt-fork"
            ),
            "{}",
            binding.warnings[0]
        );
        assert!(host.release("first").is_empty());
        assert_eq!(
            rig.chat(),
            json!({ "ctrl+e": "chat:externalEditor", "ctrl+x ctrl+c": "chat:stash" })
        );
    }

    #[test]
    fn the_next_launch_takes_over_what_a_launch_that_never_released_left() {
        let rig = Rig::new();
        let host = rig.host(Residency::Portable);
        rig.bind(&host, "first");

        assert_eq!(rig.bind(&host, "second").bound, wanted());
        assert!(host.release("first").is_empty());
        assert_eq!(rig.chat().as_object().unwrap().len(), 2);

        assert!(host.release("second").is_empty());
        assert!(!rig.file().exists());
    }

    #[test]
    fn a_launch_that_fails_hands_the_chords_back_to_the_switchboard_it_took_them_from() {
        let rig = Rig::new();
        let host = rig.host(Residency::Portable);
        rig.bind(&host, "live");
        rig.bind(&host, "failed");

        assert!(host.hand_back("failed").is_empty());
        assert!(host.release("failed").is_empty());
        assert_eq!(rig.chat().as_object().unwrap().len(), 2);

        assert!(host.release("live").is_empty());
        assert!(!rig.file().exists());
    }

    #[test]
    fn a_launch_that_fails_takes_out_the_chords_it_added_itself() {
        let rig = Rig::new();
        let host = rig.host(Residency::Portable);
        rig.bind(&host, "failed");

        assert!(host.hand_back("failed").is_empty());

        assert!(!rig.file().exists());
    }

    #[test]
    fn a_file_that_is_not_key_bindings_is_left_alone() {
        let rig = Rig::new();
        let host = rig.host(Residency::Portable);
        std::fs::write(rig.file(), "[]").unwrap();
        let binder = rig.interpreter.chord_binder_for(&rig.tool).unwrap();

        let error = bind(
            &host,
            binder,
            &rig.tool,
            &rig.dir.path().join("home"),
            "first",
        )
        .err()
        .unwrap();

        assert!(
            format!("{error:#}").contains("is left as it is"),
            "{error:#}"
        );
        assert_eq!(std::fs::read_to_string(rig.file()).unwrap(), "[]");
    }

    #[test]
    fn the_plugin_is_handed_the_launch_and_each_bound_chord_as_it_is_typed() {
        let bound = Bound {
            launch: "9f-1a".to_owned(),
            chords: wanted(),
        };
        assert_eq!(
            serde_json::from_str::<Value>(&bound.handoff()).unwrap(),
            json!({ "launch": "9f-1a", "chords": [
                { "keys": "ctrl+x ctrl+x", "command": "close-fork" },
                { "keys": "ctrl+x ctrl+c", "command": "interrupt-fork" },
            ] })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&Bound::default().handoff()).unwrap(),
            json!({ "launch": "", "chords": [] })
        );
    }

    #[test]
    fn the_release_hook_needs_a_launch_id() {
        assert!(run_release(&[]).unwrap_err().to_string().contains(RELEASE));
    }
}
