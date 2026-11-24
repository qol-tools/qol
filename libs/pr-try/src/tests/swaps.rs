use super::super::*;
use std::fs;

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const BRANCH: &str = "feat/demo";
const PLUGIN: &str = "qol-demo";

struct Fake {
    number: u64,
    tray: bool,
}

impl Source for Fake {
    fn pull_request(&self, _number: u64) -> Result<PullRequest> {
        Ok(PullRequest {
            head_sha: HEAD.into(),
            head_branch: BRANCH.into(),
            from_fork: false,
        })
    }

    fn live_test_runs(&self, _pull: &PullRequest) -> Result<Vec<Run>> {
        Ok(vec![Run {
            id: 7,
            status: "completed".into(),
            conclusion: "success".into(),
            head_branch: BRANCH.into(),
            url: "https://example.invalid/7".into(),
        }])
    }

    fn job_succeeded(&self, _run: &Run, _job: &str) -> Result<bool> {
        Ok(true)
    }

    fn download(&self, _run: &Run, _artifact: &str, dir: &Path) -> Result<()> {
        let build = host_build()?;
        let tag = format!("{PLUGIN}-v1.1.0");
        let file = format!("{PLUGIN}-build");
        let manifest = serde_json::json!({
            "schema": 1,
            "pull_request": self.number,
            "head_sha": HEAD,
            "target": build.target,
            "plugins": [{
                "id": PLUGIN,
                "version": "1.1.0",
                "tag": tag,
                "file": file,
                "bin_name": PLUGIN
            }],
            "tray": if self.tray { TRAY_BINARY } else { "" }
        });
        fs::write(dir.join("manifest.json"), manifest.to_string())?;
        let plugins = dir.join("plugins").join(&tag);
        fs::create_dir_all(&plugins)?;
        fs::copy(std::env::current_exe()?, plugins.join(&file))?;
        if self.tray {
            fs::create_dir_all(dir.join("tray"))?;
            fs::copy(std::env::current_exe()?, dir.join("tray").join(TRAY_BINARY))?;
        }
        Ok(())
    }

    fn open_pulls(&self, _job: &str) -> Result<Vec<OpenPull>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct Recorder {
    lines: Vec<String>,
    events: Vec<String>,
    record_at: Option<PathBuf>,
    caught_up: Vec<Vec<String>>,
    hand_over_fails: bool,
    tray_already_runs: bool,
}

impl Host for Recorder {
    fn say(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }

    fn run_tray(&mut self, binary: &Path) -> Result<String> {
        let saved = self
            .record_at
            .as_deref()
            .filter(|path| path.is_file())
            .map(|path| State::load(path).unwrap())
            .map(|state| (state.tray.is_some(), state.installed_tray.is_some()));
        self.events
            .push(format!("run {} saved={saved:?}", binary.display()));
        Ok("runs".into())
    }

    fn put_back_tray(
        &mut self,
        try_binary: &Path,
        installed: &Path,
        settle: &mut dyn FnMut() -> Result<()>,
    ) -> Result<TrayBack> {
        settle()?;
        if self.hand_over_fails {
            anyhow::bail!("the installed tray did not start");
        }
        self.events.push(format!(
            "put back {} for {}",
            installed.display(),
            try_binary.display()
        ));
        Ok(TrayBack {
            said: "back".into(),
            started: !self.tray_already_runs,
        })
    }

    fn running_install(&self, _downloads: &Path) -> Result<Option<PathBuf>> {
        Ok(Some(PathBuf::from("/installed/qol-tray")))
    }

    fn catch_up(&mut self, put_back: &[String]) {
        self.caught_up.push(put_back.to_vec());
    }
}

struct Machine {
    _root: tempfile::TempDir,
    places: Places,
    plugin: PathBuf,
}

fn machine() -> Machine {
    let root = tempfile::tempdir().unwrap();
    let places = Places::under(
        &root.path().join("config"),
        &root.path().join("data"),
        &root.path().join("cache"),
    );
    let plugin = places.plugins.join(PLUGIN).join(PLUGIN);
    fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    fs::write(&plugin, b"installed").unwrap();
    Machine {
        _root: root,
        places,
        plugin,
    }
}

fn tried(number: u64, keep_tray: bool) -> Try {
    Try { number, keep_tray }
}

#[test]
fn a_plugin_build_is_swapped_in_and_off_puts_the_installed_one_back() {
    let at = machine();
    let mut host = Recorder::default();
    let fake = Fake {
        number: 412,
        tray: false,
    };
    let outcome = try_build(
        &fake,
        &mut host,
        tried(412, true),
        host_build().unwrap(),
        &at.places,
    )
    .unwrap();
    assert_eq!(outcome, Tried::Installed);
    assert_ne!(fs::read(&at.plugin).unwrap(), b"installed");
    let record = State::load(&at.places.state).unwrap();
    assert_eq!(record.pull_request, Some(412));
    assert_eq!(record.tray, None);
    assert!(host.lines[0].starts_with("pull request #412 at 012345678"));
    assert!(host.events.is_empty(), "{:?}", host.events);

    let put_back = turn_off(&mut host, &at.places).unwrap();
    assert_eq!(put_back, [PLUGIN]);
    assert_eq!(fs::read(&at.plugin).unwrap(), b"installed");
    assert!(!at.places.state.exists());
    assert_eq!(
        turn_off(&mut host, &at.places).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        host.lines.last().unwrap(),
        "no pull request build is installed"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_tray_build_is_recorded_before_the_tray_hands_over_and_cleared_before_it_comes_back() {
    let at = machine();
    let mut host = Recorder {
        record_at: Some(at.places.state.clone()),
        ..Recorder::default()
    };
    let fake = Fake {
        number: 412,
        tray: true,
    };
    let refused = try_build(
        &fake,
        &mut host,
        tried(412, true),
        host_build().unwrap(),
        &at.places,
    )
    .unwrap_err();
    assert_eq!(
        refused.downcast_ref::<RestartsTray>(),
        Some(&RestartsTray::Carries(412))
    );
    try_build(
        &fake,
        &mut host,
        tried(412, false),
        host_build().unwrap(),
        &at.places,
    )
    .unwrap();
    assert_eq!(host.events.len(), 1);
    assert!(
        host.events[0].ends_with("saved=Some((true, true))"),
        "{:?}",
        host.events
    );
    let record = State::load(&at.places.state).unwrap();
    assert_eq!(
        record.installed_tray,
        Some(PathBuf::from("/installed/qol-tray"))
    );
    assert!(host
        .lines
        .iter()
        .any(|line| line.ends_with("installed; the tray build starts it")));

    let put_back = turn_off(&mut host, &at.places).unwrap();
    assert_eq!(put_back, [PLUGIN, TRAY_BINARY]);
    assert!(host.events[1].starts_with("put back /installed/qol-tray"));
    assert!(!at.places.state.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn a_plugin_only_build_after_a_tray_build_brings_the_installed_tray_back_and_keeps_its_plugins() {
    let at = machine();
    let mut host = Recorder::default();
    let build = host_build().unwrap();
    let with_tray = Fake {
        number: 412,
        tray: true,
    };
    try_build(&with_tray, &mut host, tried(412, false), build, &at.places).unwrap();
    let plugins_only = Fake {
        number: 413,
        tray: false,
    };
    let refused = try_build(
        &plugins_only,
        &mut host,
        tried(413, true),
        build,
        &at.places,
    )
    .unwrap_err();
    assert_eq!(
        refused.downcast_ref::<RestartsTray>(),
        Some(&RestartsTray::Running {
            tried: 413,
            running: 412
        })
    );
    try_build(
        &plugins_only,
        &mut host,
        tried(413, false),
        build,
        &at.places,
    )
    .unwrap();
    let record = State::load(&at.places.state).unwrap();
    assert_eq!(record.pull_request, Some(413));
    assert_eq!(record.tray, None);
    assert_eq!(record.installed_tray, None);
    assert_eq!(record.plugins.len(), 1);
    assert!(record.is_active());
    assert!(host.events[1].starts_with("put back /installed/qol-tray"));
    assert_eq!(host.caught_up, [Vec::<String>::new()]);
    assert!(host
        .lines
        .iter()
        .any(|line| line.ends_with("installed; the installed tray starts it")));
}

#[cfg(target_os = "linux")]
#[test]
fn plugins_are_restarted_when_putting_the_tray_back_started_none() {
    let at = machine();
    let mut host = Recorder {
        tray_already_runs: true,
        ..Recorder::default()
    };
    let build = host_build().unwrap();
    let with_tray = Fake {
        number: 412,
        tray: true,
    };
    try_build(&with_tray, &mut host, tried(412, false), build, &at.places).unwrap();
    let plugins_only = Fake {
        number: 413,
        tray: false,
    };
    try_build(
        &plugins_only,
        &mut host,
        tried(413, false),
        build,
        &at.places,
    )
    .unwrap();
    let restarted = format!("  {PLUGIN}: installed; no daemon was running");
    assert_eq!(
        host.lines.iter().filter(|line| **line == restarted).count(),
        1,
        "{:?}",
        host.lines
    );

    try_build(&with_tray, &mut host, tried(412, false), build, &at.places).unwrap();
    host.lines.clear();
    turn_off(&mut host, &at.places).unwrap();
    assert!(host.lines.contains(&restarted), "{:?}", host.lines);
}

#[cfg(target_os = "linux")]
#[test]
fn a_switch_from_a_plugin_only_build_restarts_what_it_put_back_before_a_tray_build() {
    let at = machine();
    let mut host = Recorder::default();
    let build = host_build().unwrap();
    let plugins_only = Fake {
        number: 412,
        tray: false,
    };
    try_build(
        &plugins_only,
        &mut host,
        tried(412, true),
        build,
        &at.places,
    )
    .unwrap();
    let with_tray = Fake {
        number: 413,
        tray: true,
    };
    try_build(&with_tray, &mut host, tried(413, false), build, &at.places).unwrap();
    assert!(
        host.lines.contains(&format!(
            "  {PLUGIN}: put back; installed; no daemon was running"
        )),
        "{:?}",
        host.lines
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_failed_hand_over_keeps_the_record_so_off_can_run_again() {
    let at = machine();
    let mut host = Recorder::default();
    let build = host_build().unwrap();
    let with_tray = Fake {
        number: 412,
        tray: true,
    };
    try_build(&with_tray, &mut host, tried(412, false), build, &at.places).unwrap();
    host.hand_over_fails = true;
    let error = format!("{:#}", turn_off(&mut host, &at.places).unwrap_err());
    assert!(error.contains("did not start"), "{error}");
    let kept = State::load(&at.places.state).unwrap();
    assert_eq!(kept.pull_request, Some(412));
    assert!(kept.tray.is_some());
    assert!(kept.plugins.is_empty());

    host.hand_over_fails = false;
    turn_off(&mut host, &at.places).unwrap();
    assert!(!at.places.state.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn a_failed_hand_over_after_a_plugin_only_build_keeps_the_tray_build_recorded() {
    let at = machine();
    let mut host = Recorder::default();
    let build = host_build().unwrap();
    let with_tray = Fake {
        number: 412,
        tray: true,
    };
    try_build(&with_tray, &mut host, tried(412, false), build, &at.places).unwrap();
    host.hand_over_fails = true;
    let plugins_only = Fake {
        number: 413,
        tray: false,
    };
    try_build(
        &plugins_only,
        &mut host,
        tried(413, false),
        build,
        &at.places,
    )
    .unwrap_err();
    let kept = State::load(&at.places.state).unwrap();
    assert_eq!(kept.pull_request, Some(413));
    assert!(kept.tray.is_some());
    assert!(kept.installed_tray.is_some());
    assert_eq!(kept.plugins.len(), 1);

    host.hand_over_fails = false;
    assert_eq!(
        turn_off(&mut host, &at.places).unwrap(),
        [PLUGIN, TRAY_BINARY]
    );
    assert!(!at.places.state.exists());
}

#[test]
fn a_new_build_of_the_tried_pull_request_puts_back_the_plugins_it_no_longer_changes() {
    let at = machine();
    let mut host = Recorder::default();
    let build = host_build().unwrap();
    let fake = Fake {
        number: 412,
        tray: false,
    };
    try_build(&fake, &mut host, tried(412, true), build, &at.places).unwrap();
    let gone = at.places.plugins.join("qol-gone").join("qol-gone");
    let backup = at.places.backup_for("qol-gone", "qol-gone");
    for (path, text) in [(&gone, "tried"), (&backup, "installed")] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let mut record = State::load(&at.places.state).unwrap();
    record.plugins.insert(
        0,
        install::Swapped {
            id: "qol-gone".into(),
            target: gone.clone(),
            backup,
        },
    );
    record.save(&at.places.state).unwrap();

    try_build(&fake, &mut host, tried(412, true), build, &at.places).unwrap();
    assert_eq!(fs::read(&gone).unwrap(), b"installed");
    assert_ne!(fs::read(&at.plugin).unwrap(), b"installed");
    let record = State::load(&at.places.state).unwrap();
    let ids: Vec<_> = record
        .plugins
        .iter()
        .map(|swapped| swapped.id.as_str())
        .collect();
    assert_eq!(ids, [PLUGIN]);
    assert_eq!(host.caught_up, [vec!["qol-gone".to_string()]]);
    assert!(host
        .lines
        .iter()
        .any(|line| line == "putting back the plugins #412's new build no longer changes"));

    assert_eq!(turn_off(&mut host, &at.places).unwrap(), [PLUGIN]);
    assert_eq!(fs::read(&at.plugin).unwrap(), b"installed");
}

struct Building;

impl Source for Building {
    fn pull_request(&self, number: u64) -> Result<PullRequest> {
        Fake {
            number,
            tray: false,
        }
        .pull_request(number)
    }

    fn live_test_runs(&self, _pull: &PullRequest) -> Result<Vec<Run>> {
        Ok(vec![Run {
            id: 8,
            status: "in_progress".into(),
            conclusion: String::new(),
            head_branch: BRANCH.into(),
            url: "https://example.invalid/8".into(),
        }])
    }

    fn job_succeeded(&self, _run: &Run, _job: &str) -> Result<bool> {
        Ok(false)
    }

    fn download(&self, _run: &Run, _artifact: &str, _dir: &Path) -> Result<()> {
        anyhow::bail!("nothing is built yet")
    }

    fn open_pulls(&self, _job: &str) -> Result<Vec<OpenPull>> {
        Ok(Vec::new())
    }
}

#[test]
fn a_newer_build_is_a_moved_head_whose_live_test_built_this_machine() {
    let build = host_build().unwrap();
    let built = Fake {
        number: 412,
        tray: false,
    };
    let cases: [(&str, &dyn Source, &str, Option<&str>); 3] = [
        ("the tried head is the newest", &built, HEAD, None),
        ("the head moved and is built", &built, "0ld", Some(HEAD)),
        ("the head moved and still builds", &Building, "0ld", None),
    ];
    for (case, source, tried_head, expected) in cases {
        assert_eq!(
            newer_build(source, build, 412, tried_head)
                .unwrap()
                .as_deref(),
            expected,
            "case: {case}"
        );
    }
}
