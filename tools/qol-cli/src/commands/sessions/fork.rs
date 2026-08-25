use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use qol_terminal_sessions::cli::{CliSessionInterpreter, CliToolId};
use qol_terminal_sessions::TerminalSessionService;
use serde::{Deserialize, Serialize};

use super::spawn::{
    config_spawn_cap, config_surface, resolve_spawn_cap, spawn_detached, SpawnCapConfig,
    SpawnLedger, SpawnLocks,
};

const BRIEF_MAX_BYTES: usize = 256 * 1024;
const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ForkRecord {
    pub(super) key: String,
    pub(super) tool: String,
    pub(super) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    pub(super) surface: String,
    pub(super) cwd: String,
    pub(super) session: String,
    pub(super) title: String,
    pub(super) brief: String,
    pub(super) created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) parent: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ForkOutcome {
    pub(super) session: String,
    pub(super) tool: String,
    pub(super) key: String,
    pub(super) model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    pub(super) cwd: String,
    pub(super) surface: String,
    pub(super) title: String,
    pub(super) brief: String,
    pub(super) detached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) parent: Option<String>,
    pub(super) elapsed_ms: u128,
    pub(super) instruction: String,
}

pub(super) struct ForkStore {
    dir: PathBuf,
}

impl ForkStore {
    pub(super) fn system() -> Result<Self> {
        Ok(Self::with_dir(super::bridge::trace_dir().join("forks")))
    }

    pub(super) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(super) fn brief_path(&self, key: &str, created_at: u64) -> PathBuf {
        self.dir.join(format!("{}-{created_at}.md", slug(key)))
    }

    fn record_path(&self, key: &str, created_at: u64) -> PathBuf {
        self.dir.join(format!("{}-{created_at}.json", slug(key)))
    }

    pub(super) fn write_brief(&self, key: &str, created_at: u64, brief: &str) -> Result<PathBuf> {
        fs::create_dir_all(&self.dir).context("failed to create the fork directory")?;
        let path = self.brief_path(key, created_at);
        fs::write(&path, brief).context("failed to write the fork brief")?;
        Ok(path)
    }

    pub(super) fn record(&self, record: &ForkRecord) -> Result<()> {
        fs::create_dir_all(&self.dir).context("failed to create the fork directory")?;
        let path = self.record_path(&record.key, record.created_at);
        let temporary = path.with_extension("tmp");
        let encoded = serde_json::to_string(record)?;
        fs::write(&temporary, encoded).context("failed to write the fork record")?;
        fs::rename(&temporary, &path).context("failed to publish the fork record")
    }

    pub(super) fn list(&self) -> Result<Vec<ForkRecord>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("failed to read the fork directory"),
        };
        let mut records = Vec::new();
        for entry in entries {
            let path = entry.context("failed to read the fork directory")?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Ok(encoded) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<ForkRecord>(&encoded) else {
                continue;
            };
            records.push(record);
        }
        records.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.key.cmp(&right.key))
        });
        Ok(records)
    }

    pub(super) fn find_session(&self, session: &str) -> Result<Option<ForkRecord>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|record| record.session == session))
    }
}

fn slug(key: &str) -> String {
    key.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

pub(super) fn validate_brief(brief: &str) -> Result<()> {
    if brief.trim().is_empty() {
        bail!("a fork brief must not be empty: the detached architect has nothing else to go on");
    }
    if brief.len() > BRIEF_MAX_BYTES {
        bail!("fork brief exceeds {BRIEF_MAX_BYTES} bytes");
    }
    if brief
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t' | '\r'))
    {
        bail!("fork brief contains unsupported control characters");
    }
    Ok(())
}

pub(super) fn effort_args(tool: &CliToolId, effort: Option<&str>) -> Result<Vec<String>> {
    let Some(effort) = effort else {
        return Ok(Vec::new());
    };
    if !EFFORT_LEVELS.contains(&effort) {
        bail!(
            "invalid effort `{effort}`; expected one of {}",
            EFFORT_LEVELS.join(", ")
        );
    }
    match tool.as_str() {
        "claude" => Ok(vec!["--effort".to_owned(), effort.to_owned()]),
        other => {
            bail!("tool `{other}` has no effort flag; drop --effort or fork a tool that takes one")
        }
    }
}

pub(super) fn fork_prompt(brief_path: &Path, parent: Option<&str>) -> String {
    let lineage = match parent {
        Some(parent) => format!(
            "The session that forked you is `{parent}`. It has already moved on to other work and is not waiting for you."
        ),
        None => "The session that forked you has already moved on to other work and is not waiting for you.".to_owned(),
    };
    format!(
        "[qol session fork]\nYou are a detached architect and this terminal is the root of a new tree. Nothing collects your result, no round is open on you, and there is no completion marker to print. Own the problem end to end and report to the user in this terminal.\n\nYour brief is written to {}. Read it first, then work the problem.\n\n{lineage} Do not try to report back to it and do not bridge to it. Spawn your own lanes if you need them.",
        brief_path.display()
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fork(
    terminals: &TerminalSessionService,
    interpreter: &CliSessionInterpreter,
    ledger: &SpawnLedger,
    locks: &SpawnLocks,
    forks: &ForkStore,
    tool: &str,
    cwd: &str,
    key: &str,
    surface: Option<&str>,
    model: &str,
    effort: Option<&str>,
    title: Option<&str>,
    brief: &str,
    parent: Option<&str>,
    cap: Option<&SpawnCapConfig>,
) -> Result<ForkOutcome> {
    validate_brief(brief)?;
    let tool_id = CliToolId::new(tool.to_owned())
        .map_err(|error| anyhow!("invalid tool `{tool}`: {error}"))?;
    let extra = effort_args(&tool_id, effort)?;
    let created_at = now_seconds();
    let brief_path = forks.write_brief(key, created_at, brief)?;
    let prompt = fork_prompt(&brief_path, parent);
    let launched = spawn_detached(
        terminals,
        interpreter,
        ledger,
        locks,
        tool,
        cwd,
        key,
        surface,
        Some(model),
        &extra,
        title,
        config_surface()?,
        cap,
        &prompt,
    )?;
    let record = ForkRecord {
        key: key.to_owned(),
        tool: tool.to_owned(),
        model: model.to_owned(),
        effort: effort.map(str::to_owned),
        surface: launched.surface.to_owned(),
        cwd: launched.cwd.clone(),
        session: launched.session.clone(),
        title: launched.title.clone(),
        brief: brief_path.display().to_string(),
        created_at,
        parent: parent.map(str::to_owned),
    };
    forks.record(&record)?;
    qol_runtime::probe!(
        "CLI_SESSION_FORK",
        "event=forked key={} tool={} model={} effort={} session={}",
        key,
        tool,
        model,
        effort.unwrap_or("default"),
        record.session
    );
    Ok(ForkOutcome {
        session: record.session.clone(),
        tool: record.tool.clone(),
        key: record.key.clone(),
        model: record.model.clone(),
        effort: record.effort.clone(),
        cwd: record.cwd.clone(),
        surface: record.surface.clone(),
        title: record.title.clone(),
        brief: record.brief.clone(),
        detached: true,
        parent: record.parent.clone(),
        elapsed_ms: launched.elapsed_ms,
        instruction: "The fork is detached: no round is open on it, session_bridge will refuse it, and it never reports back. Return to your own work.".to_owned(),
    })
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let parsed = parse_args(args)?;
    super::spawn::enforce_allowed_model(&parsed.model)?;
    if parsed.help {
        println!("{}", help());
        return Ok(());
    }
    let brief = match (&parsed.brief, &parsed.brief_file) {
        (Some(_), Some(_)) => bail!("pass --brief or --brief-file, not both"),
        (Some(brief), None) => brief.clone(),
        (None, Some(path)) => fs::read_to_string(path)
            .with_context(|| format!("failed to read the fork brief at {path}"))?,
        (None, None) => bail!("a fork needs --brief TEXT or --brief-file PATH"),
    };
    let terminals = super::service()?;
    let outcome = fork(
        &terminals,
        &CliSessionInterpreter::system(),
        &SpawnLedger::system()?,
        &SpawnLocks::system()?,
        &ForkStore::system()?,
        &parsed.tool,
        &parsed.cwd,
        &parsed.key,
        parsed.surface.as_deref(),
        &parsed.model,
        parsed.effort.as_deref(),
        parsed.title.as_deref(),
        &brief,
        parsed.parent.as_deref(),
        resolve_spawn_cap(config_spawn_cap()?).as_ref(),
    )?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

pub(super) fn run_list(_args: &[OsString]) -> Result<()> {
    let records = ForkStore::system()?.list()?;
    if records.is_empty() {
        println!("no detached forks recorded");
        return Ok(());
    }
    for record in records {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            record.key,
            record.tool,
            record.effort.as_deref().unwrap_or(&record.model),
            record.session,
            record.brief
        );
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ForkArgs {
    pub(super) help: bool,
    pub(super) tool: String,
    pub(super) cwd: String,
    pub(super) key: String,
    pub(super) surface: Option<String>,
    pub(super) model: String,
    pub(super) effort: Option<String>,
    pub(super) title: Option<String>,
    pub(super) brief: Option<String>,
    pub(super) brief_file: Option<String>,
    pub(super) parent: Option<String>,
}

pub(super) fn help() -> String {
    "qol sessions fork --tool TOOL --cwd PATH --key KEY [--model MODEL] (--brief TEXT | --brief-file PATH) [--effort LEVEL] [--title TITLE] [--surface tab|os-window] [--parent SESSION]\n\nLaunch a detached architect: a new terminal that owns the brief end to end and never reports back. No round is opened on it, no completion marker is embedded, and session_bridge refuses it. The brief is written to a file under the sessions data dir and the launch points the new architect at that path, so a long problem statement survives argv limits and stays readable after the screen scrolls.\n\nUse it when a second problem surfaces mid-session and chasing it would cost you the thread you are already holding: fork it away at a tier that can finish it, and carry on.\n\n--model defaults to spawn_model in sessions.toml and is refused unless allowed_models permits it, because tiers are billed per token and only the person paying picks one.\n--effort is passed to tools that take one (claude: low, medium, high, xhigh, max).\nqol sessions forks lists what has been forked.".to_owned()
}

pub(super) fn parse_args(args: &[OsString]) -> Result<ForkArgs> {
    let mut parsed = ForkArgs {
        tool: "claude".to_owned(),
        ..ForkArgs::default()
    };
    let mut index = 0;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| anyhow!("fork arguments must be valid UTF-8"))?;
        match flag {
            "help" | "--help" | "-h" => {
                parsed.help = true;
                return Ok(parsed);
            }
            "--tool" => parsed.tool = flag_value(args, &mut index, "--tool")?,
            "--cwd" => parsed.cwd = flag_value(args, &mut index, "--cwd")?,
            "--key" => parsed.key = flag_value(args, &mut index, "--key")?,
            "--surface" => parsed.surface = Some(flag_value(args, &mut index, "--surface")?),
            "--model" => parsed.model = flag_value(args, &mut index, "--model")?,
            "--effort" => parsed.effort = Some(flag_value(args, &mut index, "--effort")?),
            "--title" => parsed.title = Some(flag_value(args, &mut index, "--title")?),
            "--brief" => parsed.brief = Some(flag_value(args, &mut index, "--brief")?),
            "--brief-file" => {
                parsed.brief_file = Some(flag_value(args, &mut index, "--brief-file")?)
            }
            "--parent" => parsed.parent = Some(flag_value(args, &mut index, "--parent")?),
            other => bail!("unknown fork flag `{other}`\n\n{}", help()),
        }
        index += 1;
    }
    if parsed.cwd.is_empty() {
        bail!("fork requires --cwd\n\n{}", help());
    }
    if parsed.key.is_empty() {
        bail!(
            "fork requires --key so the detached tree is findable later\n\n{}",
            help()
        );
    }
    if parsed.model.is_empty() {
        parsed.model = super::spawn::config_spawn_model()?.unwrap_or_default();
    }
    if parsed.model.is_empty() {
        bail!(
            "fork requires --model or a spawn_model in sessions.toml so the tier is one this host may launch\n\n{}",
            help()
        );
    }
    Ok(parsed)
}

fn flag_value(args: &[OsString], index: &mut usize, flag: &str) -> Result<String> {
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| anyhow!("{flag} requires a value"))?;
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{flag} value must be valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn fork_args_default_to_claude_and_require_key_and_cwd() {
        let parsed = parse_args(&args(&[
            "--cwd",
            "/work",
            "--key",
            "chase-lockfile",
            "--model",
            "opus",
            "--effort",
            "xhigh",
            "--brief",
            "the lockfile goes stale",
        ]))
        .unwrap();
        assert_eq!(parsed.tool, "claude");
        assert_eq!(parsed.model, "opus");
        assert_eq!(parsed.effort.as_deref(), Some("xhigh"));
        assert_eq!(parsed.brief.as_deref(), Some("the lockfile goes stale"));

        for (missing, expected) in [
            (args(&["--key", "k", "--model", "opus"]), "--cwd"),
            (args(&["--cwd", "/work", "--model", "opus"]), "--key"),
        ] {
            let error = parse_args(&missing).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn effort_is_validated_and_only_offered_to_tools_that_take_it() {
        let claude = CliToolId::new("claude").unwrap();
        assert_eq!(
            effort_args(&claude, Some("xhigh")).unwrap(),
            vec!["--effort".to_owned(), "xhigh".to_owned()]
        );
        assert!(effort_args(&claude, None).unwrap().is_empty());
        let error = effort_args(&claude, Some("colossal"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid effort"), "{error}");
        let error = effort_args(&CliToolId::new("pi").unwrap(), Some("xhigh"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("no effort flag"), "{error}");
    }

    #[test]
    fn the_fork_prompt_points_at_the_brief_and_forbids_reporting_back() {
        let prompt = fork_prompt(Path::new("/data/forks/chase-1.md"), Some("v1:kitty:7:100"));
        assert!(prompt.contains("/data/forks/chase-1.md"));
        assert!(prompt.contains("detached architect"));
        assert!(prompt.contains("no completion marker"));
        assert!(prompt.contains("v1:kitty:7:100"));
        assert!(
            !prompt.contains("Completion fragments"),
            "a fork never carries a completion marker: {prompt}"
        );
    }

    #[test]
    fn an_empty_or_oversized_brief_is_refused() {
        assert!(validate_brief("   \n")
            .unwrap_err()
            .to_string()
            .contains("must not be empty"));
        assert!(validate_brief(&"x".repeat(BRIEF_MAX_BYTES + 1))
            .unwrap_err()
            .to_string()
            .contains("exceeds"));
        validate_brief("chase the stale lockfile").unwrap();
    }

    #[test]
    fn the_store_writes_a_brief_and_lists_records_newest_first() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ForkStore::with_dir(root.path().join("forks"));
        let brief = store
            .write_brief("chase-lockfile", 100, "the lockfile goes stale")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&brief).unwrap(),
            "the lockfile goes stale"
        );
        for (key, created_at) in [("older", 100_u64), ("newer", 200)] {
            store
                .record(&ForkRecord {
                    key: key.to_owned(),
                    tool: "claude".to_owned(),
                    model: "opus".to_owned(),
                    effort: Some("xhigh".to_owned()),
                    surface: "tab".to_owned(),
                    cwd: "/work".to_owned(),
                    session: format!("v1:kitty:{key}:1"),
                    title: key.to_owned(),
                    brief: brief.display().to_string(),
                    created_at,
                    parent: None,
                })
                .unwrap();
        }
        let records = store.list().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.key.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"]
        );
        assert_eq!(
            store.find_session("v1:kitty:newer:1").unwrap().unwrap().key,
            "newer"
        );
        assert!(store.find_session("v1:kitty:absent:1").unwrap().is_none());
    }
}
