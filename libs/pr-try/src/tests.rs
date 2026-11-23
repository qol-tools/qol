use super::*;
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
}

impl Host for Recorder {
    fn say(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }

    fn run_tray(&mut self, binary: &Path) -> Result<String> {
        let saved = self.record_at.as_deref().is_some_and(Path::is_file);
        self.events
            .push(format!("run {} saved={saved}", binary.display()));
        Ok("runs".into())
    }

    fn put_back_tray(
        &mut self,
        try_binary: &Path,
        installed: &Path,
        settle: &mut dyn FnMut() -> Result<()>,
    ) -> Result<String> {
        settle()?;
        if self.hand_over_fails {
            anyhow::bail!("the installed tray did not start");
        }
        self.events.push(format!(
            "put back {} for {}",
            installed.display(),
            try_binary.display()
        ));
        Ok("back".into())
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
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
    assert!(host.events[0].ends_with("saved=true"), "{:?}", host.events);
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
fn keep_tray_refuses_while_a_tray_build_runs_whichever_pull_request_it_came_from() {
    let running = State {
        pull_request: Some(412),
        tray: Some("/t/qol-tray".into()),
        ..State::default()
    };
    let plugins_only = State {
        pull_request: Some(412),
        ..State::default()
    };
    let refused = |tried| {
        Some(RestartsTray::Running {
            tried,
            running: 412,
        })
    };
    assert_eq!(running_tray_refusal(413, true, &running), refused(413));
    assert_eq!(running_tray_refusal(412, true, &running), refused(412));
    assert_eq!(running_tray_refusal(413, false, &running), None);
    assert_eq!(running_tray_refusal(413, true, &plugins_only), None);
    assert_eq!(running_tray_refusal(413, true, &State::default()), None);
}

#[test]
fn the_refusals_name_the_pull_requests() {
    assert!(RestartsTray::Carries(412)
        .to_string()
        .contains("#412 also replaces qol-tray"));
    assert!(RestartsTray::Running {
        tried: 413,
        running: 412
    }
    .to_string()
    .contains("runs pull request #412's build"));
}

#[test]
fn the_installed_tray_is_the_first_one_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let user = root
        .path()
        .join("home/Applications/QoL Tray.app/Contents/MacOS/qol-tray");
    let system = root
        .path()
        .join("Applications/QoL Tray.app/Contents/MacOS/qol-tray");
    let binaries = [user.clone(), system.clone()];
    assert_eq!(first_installed(&binaries), Some(user.clone()));
    fs::create_dir_all(system.parent().unwrap()).unwrap();
    fs::write(&system, b"tray").unwrap();
    assert_eq!(first_installed(&binaries), Some(system.clone()));
    fs::create_dir_all(user.parent().unwrap()).unwrap();
    fs::write(&user, b"tray").unwrap();
    assert_eq!(first_installed(&binaries), Some(user));
    assert_eq!(first_installed(&[]), None);
}

#[test]
fn a_recorded_install_wins_over_the_default_places() {
    let recorded = PathBuf::from("/Applications/QoL Tray.app/Contents/MacOS/qol-tray");
    let state = State {
        installed_tray: Some(recorded.clone()),
        ..State::default()
    };
    assert_eq!(installed_tray(&state).unwrap(), recorded);
}

#[test]
fn plain_name_refuses_paths() {
    for good in ["qol-cli-sessions", "qol-cli-sessions-v0.127.1"] {
        assert!(plain_name(good), "{good}");
    }
    for bad in ["", ".", "..", "a/b", "../a", "a\\b", "/abs"] {
        assert!(!plain_name(bad), "{bad}");
    }
}
