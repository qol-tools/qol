use std::process::Command;

const APPLET_SCHEMA: &str = "org.cinnamon";
const ENABLED_APPLETS_KEY: &str = "enabled-applets";

pub fn warm_applet_uuids() -> &'static [&'static str] {
    &["brightness-and-gamma-applet", "redshift"]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub original: String,
}

pub trait SettingsRunner: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> bool;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GsettingsRunner;

impl SettingsRunner for GsettingsRunner {
    fn get(&self, key: &str) -> Option<String> {
        let output = Command::new("gsettings")
            .args(["get", APPLET_SCHEMA, key])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn set(&self, key: &str, value: &str) -> bool {
        Command::new("gsettings")
            .args(["set", APPLET_SCHEMA, key, value])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

pub fn find_warm_entries(value: &str) -> Vec<String> {
    quoted_entries(value)
        .into_iter()
        .filter(|(_, _, entry)| is_warm_entry(entry))
        .map(|(_, _, entry)| entry)
        .collect()
}

pub fn without_warm_entries(value: &str) -> Option<String> {
    let entries = quoted_entries(value);
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (index, (_, _, entry)) in entries.iter().enumerate() {
        if !is_warm_entry(entry) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.1 + 1 == index => run.1 = index,
            _ => runs.push((index, index)),
        }
    }
    if runs.is_empty() {
        return None;
    }
    let cuts: Vec<(usize, usize)> = runs
        .iter()
        .map(|(first, last)| match entries.get(last + 1) {
            Some((next_start, _, _)) => (entries[*first].0, *next_start),
            None if *first > 0 => (entries[*first - 1].1, entries[*last].1),
            None => (entries[*first].0, entries[*last].1),
        })
        .collect();
    let mut stripped = String::with_capacity(value.len());
    let mut cursor = 0;
    for (start, end) in cuts {
        stripped.push_str(&value[cursor..start]);
        cursor = end;
    }
    stripped.push_str(&value[cursor..]);
    Some(stripped)
}

fn is_warm_entry(entry: &str) -> bool {
    warm_applet_uuids().iter().any(|uuid| entry.contains(uuid))
}

#[cfg(test)]
pub(crate) fn quoted_list(entries: &[&str]) -> String {
    let quoted: Vec<String> = entries.iter().map(|entry| format!("'{entry}'")).collect();
    format!("[{}]", quoted.join(", "))
}

fn quoted_entries(value: &str) -> Vec<(usize, usize, String)> {
    let characters: Vec<(usize, char)> = value.char_indices().collect();
    let mut entries = Vec::new();
    let mut cursor = 0;
    while cursor < characters.len() {
        if characters[cursor].1 != '\'' {
            cursor += 1;
            continue;
        }
        let start = characters[cursor].0;
        cursor += 1;
        let mut entry = String::new();
        let mut end = None;
        while cursor < characters.len() {
            let (index, character) = characters[cursor];
            cursor += 1;
            match character {
                '\\' if cursor < characters.len() => {
                    let (_, escaped) = characters[cursor];
                    cursor += 1;
                    entry.push(escaped);
                }
                '\'' => {
                    end = Some(index + 1);
                    break;
                }
                _ => entry.push(character),
            }
        }
        let Some(end) = end else {
            break;
        };
        entries.push((start, end, entry));
    }
    entries
}

pub struct AppletOwner<R: SettingsRunner> {
    runner: R,
}

impl<R: SettingsRunner> AppletOwner<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn detect(&self) -> Option<Claim> {
        let original = self.runner.get(ENABLED_APPLETS_KEY)?;
        if find_warm_entries(&original).is_empty() {
            return None;
        }
        Some(Claim { original })
    }

    pub fn disable(&self) -> Option<Claim> {
        let claim = self.detect()?;
        let stripped = without_warm_entries(&claim.original)?;
        self.runner
            .set(ENABLED_APPLETS_KEY, &stripped)
            .then_some(claim)
    }

    pub fn restore(&self, claim: &str) -> bool {
        self.runner.set(ENABLED_APPLETS_KEY, claim)
    }
}

pub trait AppletControl: Send + Sync {
    fn detect(&self) -> Option<Claim>;
    fn disable(&self) -> Option<Claim>;
    fn restore(&self, claim: &str) -> bool;
}

pub struct NoAppletControl;

impl AppletControl for NoAppletControl {
    fn detect(&self) -> Option<Claim> {
        None
    }

    fn disable(&self) -> Option<Claim> {
        None
    }

    fn restore(&self, _claim: &str) -> bool {
        true
    }
}

impl<R: SettingsRunner> AppletControl for AppletOwner<R> {
    fn detect(&self) -> Option<Claim> {
        AppletOwner::detect(self)
    }

    fn disable(&self) -> Option<Claim> {
        AppletOwner::disable(self)
    }

    fn restore(&self, claim: &str) -> bool {
        AppletOwner::restore(self, claim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const MENU: &str = "panel1:left:0:menu@cinnamon.org:0";
    const APPLET: &str = "panel1:right:3:brightness-and-gamma-applet@cardsurf:6";
    const CALENDAR: &str = "panel1:right:4:calendar@cinnamon.org:7";
    const REDSHIFT: &str = "panel1:left:1:redshift@cinnamon.org:1";

    #[derive(Clone, Default)]
    struct FakeRunner {
        value: Arc<Mutex<Option<String>>>,
        writes: Arc<Mutex<Vec<String>>>,
        fail: Arc<Mutex<bool>>,
    }

    impl FakeRunner {
        fn with_value(value: &str) -> Self {
            let runner = Self::default();
            *runner.value.lock().unwrap() = Some(value.to_string());
            runner
        }

        fn failing(value: &str) -> Self {
            let runner = Self::with_value(value);
            *runner.fail.lock().unwrap() = true;
            runner
        }

        fn value(&self) -> Option<String> {
            self.value.lock().unwrap().clone()
        }

        fn writes(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl SettingsRunner for FakeRunner {
        fn get(&self, _key: &str) -> Option<String> {
            self.value.lock().unwrap().clone()
        }

        fn set(&self, _key: &str, value: &str) -> bool {
            if *self.fail.lock().unwrap() {
                return false;
            }
            self.writes.lock().unwrap().push(value.to_string());
            *self.value.lock().unwrap() = Some(value.to_string());
            true
        }
    }

    #[test]
    fn find_warm_entries_reports_every_warm_entry() {
        let warm = quoted_list(&[MENU, APPLET]);
        let calendar = quoted_list(&[MENU, APPLET, CALENDAR]);
        let redshift = quoted_list(&[MENU, REDSHIFT]);
        assert_eq!(find_warm_entries(&warm), vec![APPLET.to_string()]);
        assert_eq!(find_warm_entries(&calendar), vec![APPLET.to_string()]);
        assert_eq!(find_warm_entries(&redshift), vec![REDSHIFT.to_string()]);
        assert!(find_warm_entries(&quoted_list(&[MENU])).is_empty());
        assert!(find_warm_entries("").is_empty());
    }

    #[test]
    fn without_warm_entries_preserves_every_other_entry_byte_for_byte() {
        let calendar = quoted_list(&[MENU, APPLET, CALENDAR]);
        let calendar_stripped = quoted_list(&[MENU, CALENDAR]);
        assert_eq!(without_warm_entries(&calendar).unwrap(), calendar_stripped);
        let tight = format!("['{APPLET}','{MENU}']");
        assert_eq!(without_warm_entries(&tight).unwrap(), format!("['{MENU}']"));
        let trailing = quoted_list(&[MENU, REDSHIFT]);
        assert_eq!(
            without_warm_entries(&trailing).unwrap(),
            format!("['{MENU}']")
        );
        let all = quoted_list(&[APPLET, REDSHIFT]);
        assert_eq!(without_warm_entries(&all).unwrap(), "[]");
    }

    #[test]
    fn without_warm_entries_returns_none_without_a_match() {
        let plain = quoted_list(&[MENU]);
        assert_eq!(without_warm_entries(&plain), None);
        assert_eq!(without_warm_entries("[]"), None);
        assert_eq!(without_warm_entries(""), None);
    }

    #[test]
    fn detect_is_none_on_an_applet_free_host() {
        let plain = quoted_list(&[MENU]);
        let owner = AppletOwner::new(FakeRunner::with_value(&plain));
        assert!(owner.detect().is_none());
        let owner = AppletOwner::new(FakeRunner::default());
        assert!(owner.detect().is_none());
        assert!(owner.disable().is_none());
    }

    #[test]
    fn disable_returns_the_original_only_after_a_successful_write() {
        let warm = quoted_list(&[MENU, APPLET]);
        let plain = quoted_list(&[MENU]);
        let runner = FakeRunner::with_value(&warm);
        let owner = AppletOwner::new(runner.clone());
        let claim = owner.disable().expect("the warm entry is removable");
        assert_eq!(claim.original, warm);
        assert_eq!(runner.writes(), vec![plain.clone()]);
        assert_eq!(runner.value().unwrap(), plain);
        assert!(owner.disable().is_none());
        let failing = AppletOwner::new(FakeRunner::failing(&warm));
        assert!(failing.disable().is_none());
    }

    #[test]
    fn restore_writes_the_stored_value_and_never_panics() {
        let warm = quoted_list(&[MENU, APPLET]);
        let runner = FakeRunner::with_value("[]");
        let owner = AppletOwner::new(runner.clone());
        assert!(owner.restore(&warm));
        assert_eq!(runner.value().unwrap(), warm);
        assert!(owner.restore(&warm));
        assert_eq!(runner.writes().len(), 2);
    }
}
