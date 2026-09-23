use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use qol_config::config_dir;

pub const REGISTRY_FILE_NAME: &str = "agents.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
    Kimi,
    Pi,
}

impl Harness {
    pub const ALL: [Harness; 4] = [Harness::Claude, Harness::Codex, Harness::Kimi, Harness::Pi];

    pub fn id(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Kimi => "kimi",
            Harness::Pi => "pi",
        }
    }

    pub fn parse(text: &str) -> Option<Harness> {
        Harness::ALL
            .into_iter()
            .find(|harness| harness.id() == text)
    }

    pub fn home_env_var(self) -> &'static str {
        match self {
            Harness::Claude => "CLAUDE_CONFIG_DIR",
            Harness::Codex => "CODEX_HOME",
            Harness::Kimi => "KIMI_CODE_HOME",
            Harness::Pi => "PI_CODING_AGENT_DIR",
        }
    }

    pub fn default_home(self, user_home: &Path) -> PathBuf {
        match self {
            Harness::Claude => user_home.join(".claude"),
            Harness::Codex => user_home.join(".codex"),
            Harness::Kimi => user_home.join(".kimi-code"),
            Harness::Pi => user_home.join(".pi").join("agent"),
        }
    }

    pub fn transcripts_dir(self, home: &Path) -> Option<PathBuf> {
        match self {
            Harness::Claude => Some(home.join("projects")),
            Harness::Codex | Harness::Pi => Some(home.join("sessions")),
            Harness::Kimi => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AgentHome {
    pub harness: Harness,
    pub id: String,
    pub path: PathBuf,
    pub shared: bool,
    pub default: bool,
    pub declared: bool,
}

pub fn normalize(text: &str, user_home: &Path) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let expanded = if trimmed == "~" {
        user_home.to_string_lossy().into_owned()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        user_home.join(rest).to_string_lossy().into_owned()
    } else if Path::new(trimmed).has_root() {
        trimmed.to_owned()
    } else {
        user_home.join(trimmed).to_string_lossy().into_owned()
    };
    without_trailing_separators(&expanded)
}

fn without_trailing_separators(path: &str) -> String {
    let stripped = if cfg!(windows) {
        path.trim_end_matches(['/', '\\'])
    } else {
        path.trim_end_matches('/')
    };
    if stripped.is_empty() {
        return path.to_owned();
    }
    stripped.to_owned()
}

#[derive(serde::Deserialize)]
struct RegistryFile {
    #[serde(default, rename = "home")]
    homes: Vec<DeclaredHome>,
}

#[derive(serde::Deserialize)]
struct DeclaredHome {
    harness: Option<String>,
    path: Option<String>,
    #[serde(default)]
    shared: bool,
    #[serde(default)]
    default: bool,
}

pub struct Registry {
    homes: Vec<AgentHome>,
    env_homes: HashMap<Harness, String>,
    user_home: PathBuf,
    pi_session_dir: Option<String>,
    load_error: Option<String>,
}

impl Registry {
    pub fn load() -> Registry {
        let file = config_dir().map(|dir| dir.join(REGISTRY_FILE_NAME));
        let user_home = dirs::home_dir().unwrap_or_default();
        Self::load_from(file.as_deref(), &user_home, &|name| std::env::var_os(name))
    }

    pub fn load_from(
        file: Option<&Path>,
        user_home: &Path,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Registry {
        let (declared, load_error) = declared_homes(file, user_home);
        let mut homes = declared;
        append_implicit_homes(&mut homes, user_home);
        replace_builtins_with_declared(&mut homes, user_home);
        apply_default_rule(&mut homes, user_home);
        let env_homes = Harness::ALL
            .into_iter()
            .filter_map(|harness| {
                let raw = env(harness.home_env_var())?;
                let id = normalize(raw.to_string_lossy().as_ref(), user_home);
                (!id.is_empty()).then_some((harness, id))
            })
            .collect();
        let pi_session_dir = env("PI_CODING_AGENT_SESSION_DIR")
            .map(|value| normalize(value.to_string_lossy().as_ref(), user_home))
            .filter(|value| !value.is_empty());
        Self {
            homes,
            env_homes,
            user_home: user_home.to_path_buf(),
            pi_session_dir,
            load_error,
        }
    }

    pub fn homes(&self) -> &[AgentHome] {
        &self.homes
    }

    pub fn current(&self, harness: Harness) -> AgentHome {
        if let Some(id) = self.env_homes.get(&harness) {
            if let Some(home) = self
                .homes
                .iter()
                .find(|home| home.harness == harness && &home.id == id)
            {
                return home.clone();
            }
            return AgentHome {
                harness,
                id: id.clone(),
                path: PathBuf::from(id),
                shared: false,
                default: false,
                declared: false,
            };
        }
        self.default_for(harness).clone()
    }

    pub fn default_for(&self, harness: Harness) -> &AgentHome {
        self.homes
            .iter()
            .find(|home| home.harness == harness && home.default)
            .expect("every harness has a default home")
    }

    pub fn is_shared(&self, id: &str) -> bool {
        self.homes.iter().any(|home| home.shared && home.id == id)
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.homes.iter().any(|home| home.id == id)
    }

    pub fn is_partitioned(&self) -> bool {
        for harness in Harness::ALL {
            let count = self
                .homes
                .iter()
                .filter(|home| home.harness == harness)
                .count();
            if count > 1 {
                return true;
            }
        }
        false
    }

    pub fn env_home(&self, harness: Harness) -> Option<&str> {
        self.env_homes.get(&harness).map(String::as_str)
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn resolve_caller(&self, explicit: Option<&str>) -> String {
        match explicit.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => normalize(value, &self.user_home),
            None => self.current(Harness::Claude).id,
        }
    }

    pub fn transcript_root(&self, home: &AgentHome) -> Option<PathBuf> {
        let root = home.harness.transcripts_dir(&home.path)?;
        if home.harness == Harness::Pi {
            let current = self.current(Harness::Pi);
            if current.id == home.id {
                if let Some(directory) = &self.pi_session_dir {
                    return Some(PathBuf::from(directory));
                }
            }
        }
        Some(root)
    }

    pub fn transcript_roots(&self) -> Vec<(AgentHome, PathBuf)> {
        self.homes
            .iter()
            .filter_map(|home| self.transcript_root(home).map(|root| (home.clone(), root)))
            .collect()
    }
}

fn declared_homes(file: Option<&Path>, user_home: &Path) -> (Vec<AgentHome>, Option<String>) {
    let Some(path) = file else {
        return (Vec::new(), None);
    };
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Vec::new(), None);
        }
        Err(error) => return (Vec::new(), Some(error.to_string())),
    };
    let mut seen: HashSet<(Harness, String)> = HashSet::new();
    let homes = match toml::from_str::<RegistryFile>(&content) {
        Ok(parsed) => parsed
            .homes
            .into_iter()
            .filter_map(|entry| {
                let harness = entry.harness.as_deref().and_then(Harness::parse)?;
                let raw = entry.path.as_deref().map(str::trim).unwrap_or("");
                if raw.is_empty() {
                    return None;
                }
                let id = normalize(raw, user_home);
                if !seen.insert((harness, id.clone())) {
                    return None;
                }
                Some(AgentHome {
                    harness,
                    id: id.clone(),
                    path: PathBuf::from(id),
                    shared: entry.shared,
                    default: entry.default,
                    declared: true,
                })
            })
            .collect(),
        Err(error) => return (Vec::new(), Some(error.to_string())),
    };
    (homes, None)
}

fn builtin_id(harness: Harness, user_home: &Path) -> String {
    normalize(
        harness.default_home(user_home).to_string_lossy().as_ref(),
        user_home,
    )
}

fn append_implicit_homes(homes: &mut Vec<AgentHome>, user_home: &Path) {
    for harness in Harness::ALL {
        let path = harness.default_home(user_home);
        homes.push(AgentHome {
            harness,
            id: builtin_id(harness, user_home),
            path,
            shared: harness == Harness::Pi,
            default: true,
            declared: false,
        });
    }
}

fn replace_builtins_with_declared(homes: &mut Vec<AgentHome>, user_home: &Path) {
    for harness in Harness::ALL {
        let builtin = builtin_id(harness, user_home);
        let replaced = homes
            .iter()
            .any(|home| home.harness == harness && home.declared && home.id == builtin);
        if replaced {
            homes.retain(|home| home.harness != harness || home.declared);
        }
    }
}

fn apply_default_rule(homes: &mut [AgentHome], user_home: &Path) {
    for harness in Harness::ALL {
        let declared_claim = homes
            .iter()
            .position(|home| home.harness == harness && home.declared && home.default);
        let builtin = builtin_id(harness, user_home);
        let slot = declared_claim.or_else(|| {
            homes
                .iter()
                .position(|home| home.harness == harness && home.id == builtin)
        });
        let Some(slot) = slot else {
            continue;
        };
        for (index, home) in homes.iter_mut().enumerate() {
            if home.harness == harness {
                home.default = index == slot;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name: &str| map.get(name).map(OsString::from)
    }

    fn user_home() -> &'static Path {
        Path::new("/home/tester")
    }

    fn write_registry(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(REGISTRY_FILE_NAME);
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn normalize_trims_expands_tilde_and_drops_trailing_separators() {
        let home = user_home();
        let cases = [
            ("~", "/home/tester"),
            ("~/", "/home/tester"),
            ("~/work/", "/home/tester/work"),
            ("~/work//", "/home/tester/work"),
            ("  /opt/x  ", "/opt/x"),
            ("/opt/x/", "/opt/x"),
            ("/opt/x//", "/opt/x"),
            ("relative/dir", "/home/tester/relative/dir"),
            ("relative", "/home/tester/relative"),
            ("/", "/"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize(input, home), expected, "input: {input}");
        }
    }

    #[test]
    fn harness_ids_parse_back_and_carry_the_known_paths() {
        for harness in Harness::ALL {
            assert_eq!(Harness::parse(harness.id()), Some(harness));
        }
        assert_eq!(Harness::parse("mystery"), None);
        assert_eq!(Harness::parse("Claude"), None);

        let home = user_home();
        let default_homes = [
            (Harness::Claude, home.join(".claude")),
            (Harness::Codex, home.join(".codex")),
            (Harness::Kimi, home.join(".kimi-code")),
            (Harness::Pi, home.join(".pi").join("agent")),
        ];
        for (harness, expected) in default_homes {
            assert_eq!(harness.default_home(home), expected);
        }

        let transcript_dirs = [
            (Harness::Claude, home.join("projects")),
            (Harness::Codex, home.join("sessions")),
            (Harness::Pi, home.join("sessions")),
        ];
        for (harness, expected) in transcript_dirs {
            assert_eq!(harness.transcripts_dir(home), Some(expected));
        }
        assert_eq!(Harness::Kimi.transcripts_dir(home), None);
    }

    #[test]
    fn a_missing_file_leaves_only_implicit_defaults() {
        let registry = Registry::load_from(None, user_home(), &env_from(&[]));
        let homes = registry.homes();
        assert_eq!(homes.len(), 4);
        for harness in Harness::ALL {
            let home = registry.default_for(harness);
            assert_eq!(home.harness, harness);
            assert_eq!(
                home.id,
                harness
                    .default_home(user_home())
                    .to_string_lossy()
                    .into_owned()
            );
            assert_eq!(home.path, harness.default_home(user_home()));
            assert!(home.default, "harness: {}", harness.id());
            assert_eq!(home.shared, harness == Harness::Pi);
            assert!(!home.declared, "harness: {}", harness.id());
        }
    }

    #[test]
    fn a_malformed_file_reports_a_load_error_and_yields_no_declared_homes() {
        let (_dir, path) = write_registry("not [ valid toml");
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        assert_eq!(registry.homes().len(), 4);
        assert!(registry.load_error().is_some());

        let registry = Registry::load_from(None, user_home(), &env_from(&[]));
        assert_eq!(registry.load_error(), None);
    }

    #[test]
    fn declared_entries_keep_order_skip_unknown_harnesses_and_blank_paths() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "mystery"
path = "/opt/mystery"

[[home]]
harness = "claude"
path = "   "

[[home]]
harness = "claude"
path = "~/.claude-work"

[[home]]
harness = "kimi"
path = "/opt/kimi/"
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        let ids: Vec<(String, bool, bool, bool)> = registry
            .homes()
            .iter()
            .map(|home| (home.id.clone(), home.shared, home.default, home.declared))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("/home/tester/.claude-work".to_owned(), false, false, true),
                ("/opt/kimi".to_owned(), false, false, true),
                ("/home/tester/.claude".to_owned(), false, true, false),
                ("/home/tester/.codex".to_owned(), false, true, false),
                ("/home/tester/.kimi-code".to_owned(), false, true, false),
                ("/home/tester/.pi/agent".to_owned(), true, true, false),
            ]
        );
    }

    #[test]
    fn shared_and_default_flags_flow_through_the_file() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "/work/shared-home"
shared = true

[[home]]
harness = "claude"
path = "/work/private-home"
default = true
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        assert!(registry.is_shared("/work/shared-home"));
        assert!(!registry.is_shared("/work/private-home"));
        assert!(!registry.is_shared("/nowhere"));
        assert_eq!(
            registry.default_for(Harness::Claude).id,
            "/work/private-home"
        );
        assert_eq!(
            registry.default_for(Harness::Claude).path,
            PathBuf::from("/work/private-home")
        );
        let builtin = registry
            .homes()
            .iter()
            .find(|home| home.id == "/home/tester/.claude")
            .unwrap();
        assert!(!builtin.default);
        assert!(!builtin.declared);
    }

    #[test]
    fn adding_a_declared_claude_home_keeps_the_builtin_claude_home() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "~/.claude-work"
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        let claude: Vec<(String, bool, bool, bool)> = registry
            .homes()
            .iter()
            .filter(|home| home.harness == Harness::Claude)
            .map(|home| (home.id.clone(), home.default, home.shared, home.declared))
            .collect();
        assert_eq!(
            claude,
            vec![
                ("/home/tester/.claude-work".to_owned(), false, false, true),
                ("/home/tester/.claude".to_owned(), true, false, false),
            ]
        );
        assert!(registry.is_partitioned());
        assert!(registry.is_registered("/home/tester/.claude-work"));
        assert!(registry.is_registered("/home/tester/.claude"));
    }

    #[test]
    fn a_declared_entry_at_the_builtin_path_replaces_the_implicit_home() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "~/.claude"
shared = true
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        let claude: Vec<AgentHome> = registry
            .homes()
            .iter()
            .filter(|home| home.harness == Harness::Claude)
            .cloned()
            .collect();
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].id, "/home/tester/.claude");
        assert!(claude[0].shared);
        assert!(claude[0].declared);
        assert!(claude[0].default);
    }

    #[test]
    fn is_partitioned_needs_more_than_one_home_for_some_harness() {
        let registry = Registry::load_from(None, user_home(), &env_from(&[]));
        assert!(!registry.is_partitioned());
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "~/.claude-work"
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        assert!(registry.is_partitioned());
    }

    #[test]
    fn env_home_reports_the_normalized_env_value() {
        let registry = Registry::load_from(None, user_home(), &env_from(&[]));
        assert_eq!(registry.env_home(Harness::Claude), None);

        let set = env_from(&[("CLAUDE_CONFIG_DIR", "~/.claude-work/")]);
        let registry = Registry::load_from(None, user_home(), &set);
        assert_eq!(
            registry.env_home(Harness::Claude),
            Some("/home/tester/.claude-work")
        );

        let blank = env_from(&[("CLAUDE_CONFIG_DIR", "   ")]);
        let registry = Registry::load_from(None, user_home(), &blank);
        assert_eq!(registry.env_home(Harness::Claude), None);
    }

    #[test]
    fn a_relative_declared_path_joins_onto_the_user_home() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "work/claude"
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        let home = &registry.homes()[0];
        assert_eq!(home.id, "/home/tester/work/claude");
        assert_eq!(home.path, PathBuf::from("/home/tester/work/claude"));
        assert!(home.declared);
        assert!(registry.is_registered("/home/tester/work/claude"));
    }

    #[test]
    fn a_duplicate_declared_entry_keeps_the_first() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "~/.claude-work"

[[home]]
harness = "claude"
path = "~/.claude-work"
shared = true
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        let claude: Vec<AgentHome> = registry
            .homes()
            .iter()
            .filter(|home| home.harness == Harness::Claude)
            .cloned()
            .collect();
        assert_eq!(claude.len(), 2);
        let work = claude
            .iter()
            .find(|home| home.id == "/home/tester/.claude-work")
            .unwrap();
        assert_eq!(work.id, "/home/tester/.claude-work");
        assert!(!work.shared);
        assert!(work.declared);
    }

    #[test]
    fn current_prefers_a_matching_env_home_then_falls_back_ad_hoc_then_to_the_default() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "~/.claude-work"
"#,
        );

        let matching = env_from(&[("CLAUDE_CONFIG_DIR", "~/.claude-work/")]);
        let registry = Registry::load_from(Some(&path), user_home(), &matching);
        let current = registry.current(Harness::Claude);
        assert_eq!(current.id, "/home/tester/.claude-work");
        assert_eq!(current.path, PathBuf::from("/home/tester/.claude-work"));
        assert!(!current.default);
        assert!(!current.shared);
        assert!(current.declared);

        let ad_hoc = env_from(&[("CLAUDE_CONFIG_DIR", "/tmp/other-claude")]);
        let registry = Registry::load_from(Some(&path), user_home(), &ad_hoc);
        let current = registry.current(Harness::Claude);
        assert_eq!(current.id, "/tmp/other-claude");
        assert_eq!(current.path, PathBuf::from("/tmp/other-claude"));
        assert!(!current.default);
        assert!(!current.shared);
        assert!(!current.declared);

        let blank = env_from(&[("CLAUDE_CONFIG_DIR", "   ")]);
        let registry = Registry::load_from(Some(&path), user_home(), &blank);
        assert_eq!(
            registry.current(Harness::Claude).id,
            registry.default_for(Harness::Claude).id
        );
    }

    #[test]
    fn resolve_caller_normalizes_the_explicit_value_or_uses_the_current_claude_home() {
        let registry = Registry::load_from(None, user_home(), &env_from(&[]));
        assert_eq!(
            registry.resolve_caller(Some("~/work/")),
            "/home/tester/work"
        );
        assert_eq!(
            registry.resolve_caller(Some("  ")),
            registry.current(Harness::Claude).id
        );
        assert_eq!(
            registry.resolve_caller(None),
            registry.current(Harness::Claude).id
        );
    }

    #[test]
    fn transcript_roots_pair_every_home_with_its_transcripts_dir() {
        let (_dir, path) = write_registry(
            r#"
[[home]]
harness = "claude"
path = "~/.claude-work"
"#,
        );
        let registry = Registry::load_from(Some(&path), user_home(), &env_from(&[]));
        let roots = registry.transcript_roots();
        let ids: Vec<String> = roots.iter().map(|(home, _)| home.id.clone()).collect();
        assert_eq!(
            ids,
            vec![
                "/home/tester/.claude-work".to_owned(),
                "/home/tester/.claude".to_owned(),
                "/home/tester/.codex".to_owned(),
                "/home/tester/.pi/agent".to_owned(),
            ]
        );
        let roots: Vec<String> = roots
            .into_iter()
            .map(|(_, root)| root.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            roots,
            vec![
                "/home/tester/.claude-work/projects".to_owned(),
                "/home/tester/.claude/projects".to_owned(),
                "/home/tester/.codex/sessions".to_owned(),
                "/home/tester/.pi/agent/sessions".to_owned(),
            ]
        );
    }

    #[test]
    fn the_pi_session_dir_override_redirects_the_current_pi_root() {
        let override_env = env_from(&[("PI_CODING_AGENT_SESSION_DIR", "~/relay-sessions/")]);
        let registry = Registry::load_from(None, user_home(), &override_env);
        let roots: Vec<String> = registry
            .transcript_roots()
            .into_iter()
            .map(|(_, root)| root.to_string_lossy().into_owned())
            .collect();
        assert!(roots.contains(&"/home/tester/relay-sessions".to_owned()));
        assert!(!roots.contains(&"/home/tester/.pi/agent/sessions".to_owned()));

        let registry = Registry::load_from(None, user_home(), &env_from(&[]));
        let roots: Vec<String> = registry
            .transcript_roots()
            .into_iter()
            .map(|(_, root)| root.to_string_lossy().into_owned())
            .collect();
        assert!(roots.contains(&"/home/tester/.pi/agent/sessions".to_owned()));
    }
}
