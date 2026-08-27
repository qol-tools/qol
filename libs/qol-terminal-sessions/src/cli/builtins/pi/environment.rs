use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use super::metadata::id_from_path;

pub fn session_file_containing_marker(
    cwd: &str,
    marker: &str,
    since: SystemTime,
) -> Option<PathBuf> {
    let directory = session_dir(cwd)?;
    let since = since.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let mut candidates: Vec<PathBuf> = session_files(&directory)
        .into_iter()
        .filter(|path| modified_unix_secs(path).is_some_and(|modified| modified >= since))
        .filter(|path| {
            std::fs::read(path).ok().is_some_and(|bytes| {
                bytes
                    .windows(marker.len())
                    .any(|window| window == marker.as_bytes())
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        match (
            created_at_unix_secs(left).map(|created| (created - since).abs()),
            created_at_unix_secs(right).map(|created| (created - since).abs()),
        ) {
            (Some(left_distance), Some(right_distance)) => left_distance.cmp(&right_distance),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| modified_unix_secs(right).cmp(&modified_unix_secs(left)))
        .then_with(|| left.cmp(right))
    });
    candidates.into_iter().next()
}

fn modified_unix_secs(path: &Path) -> Option<i64> {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|modified| modified.as_secs() as i64)
}

const START_MATCH_TOLERANCE_SECS: i64 = 60;

pub(super) trait PiEnvironment: Send + Sync {
    fn session_file(&self, pid: i32, cwd: &str) -> Option<PathBuf>;

    fn session_file_authoritative(&self, _pid: i32, _cwd: &str) -> Option<PathBuf> {
        None
    }

    fn session_files(&self, pid: i32, cwd: &str) -> Vec<PathBuf> {
        self.session_file(pid, cwd).into_iter().collect()
    }

    fn session_directory(&self, _pid: i32, cwd: &str) -> Option<PathBuf> {
        session_dir(cwd)
    }
}

pub(super) struct SystemPiEnvironment;

impl PiEnvironment for SystemPiEnvironment {
    fn session_file(&self, pid: i32, cwd: &str) -> Option<PathBuf> {
        let directory = session_dir(cwd)?;
        if let Some(path) = session_file_from_pi_env(pid, &directory) {
            return Some(path);
        }
        match process_start_unix_secs(pid) {
            Some(started_at) => session_file_started_at(&directory, started_at),
            None => newest_session_file(&directory),
        }
    }

    fn session_files(&self, pid: i32, cwd: &str) -> Vec<PathBuf> {
        let Some(directory) = session_dir(cwd) else {
            return Vec::new();
        };
        if let Some(path) = session_file_from_pi_env(pid, &directory) {
            return vec![path];
        }
        match process_start_unix_secs(pid) {
            Some(started_at) => session_files_started_at(&directory, started_at),
            None => newest_session_file(&directory).into_iter().collect(),
        }
    }

    fn session_file_authoritative(&self, pid: i32, cwd: &str) -> Option<PathBuf> {
        let directory = session_dir(cwd)?;
        session_file_from_pi_env(pid, &directory)
    }
}

fn session_file_from_pi_env(pid: i32, directory: &Path) -> Option<PathBuf> {
    session_file_from_pi_env_with(pid, directory, &read_pi_env, &child_pids)
}

fn session_file_from_pi_env_with(
    pid: i32,
    directory: &Path,
    read_env: &dyn Fn(i32) -> Option<String>,
    children: &dyn Fn(i32) -> Vec<i32>,
) -> Option<PathBuf> {
    let pi_pid = process_with_pi_env(pid, read_env, children, 0)?;
    let file = read_env(pi_pid)?;
    let path = PathBuf::from(file);
    if path.extension().is_some_and(|ext| ext == "jsonl")
        && path.parent().is_some_and(|parent| parent == directory)
    {
        return Some(path);
    }
    None
}

fn process_with_pi_env(
    pid: i32,
    read_env: &dyn Fn(i32) -> Option<String>,
    children: &dyn Fn(i32) -> Vec<i32>,
    depth: usize,
) -> Option<i32> {
    if read_env(pid).is_some() {
        return Some(pid);
    }
    if depth >= 3 {
        return None;
    }
    for child in children(pid) {
        if let Some(found) = process_with_pi_env(child, read_env, children, depth + 1) {
            return Some(found);
        }
    }
    None
}

fn read_pi_env(pid: i32) -> Option<String> {
    let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    env.split(|byte| *byte == 0).find_map(|entry| {
        let entry = std::str::from_utf8(entry).ok()?;
        entry.strip_prefix("PI_SESSION_FILE=").map(str::to_owned)
    })
}

fn child_pids(pid: i32) -> Vec<i32> {
    let Ok(children) = std::fs::read(format!("/proc/{pid}/task/{pid}/children")) else {
        return Vec::new();
    };
    children
        .split(|byte| byte.is_ascii_whitespace())
        .filter_map(|part| std::str::from_utf8(part).ok()?.parse().ok())
        .collect()
}

fn session_dir(cwd: &str) -> Option<PathBuf> {
    let base = match session_dir_override() {
        Some(base) => base,
        None => agent_dir()?.join("sessions"),
    };
    Some(base.join(session_dir_name(cwd)))
}

fn session_dir_override() -> Option<PathBuf> {
    let dir = std::env::var_os("PI_CODING_AGENT_SESSION_DIR")?;
    expand_tilde(PathBuf::from(dir))
}

fn agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        let dir = PathBuf::from(dir);
        return expand_tilde(dir);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".pi").join("agent"))
}

fn expand_tilde(path: PathBuf) -> Option<PathBuf> {
    let text = path.to_str()?;
    if text == "~" {
        return std::env::var_os("HOME").map(PathBuf::from);
    }
    let Some(rest) = text.strip_prefix("~/") else {
        return Some(path);
    };
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(rest))
}

fn session_dir_name(cwd: &str) -> String {
    let trimmed = cwd.trim_start_matches(['/', '\\']);
    let encoded: String = trimmed
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' => '-',
            other => other,
        })
        .collect();
    format!("--{encoded}--")
}

fn session_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter(|path| path.is_file())
        .collect()
}

fn session_file_started_at(directory: &Path, started_at: i64) -> Option<PathBuf> {
    session_files_started_at(directory, started_at)
        .into_iter()
        .next()
}

fn session_files_started_at(directory: &Path, started_at: i64) -> Vec<PathBuf> {
    let mut candidates: Vec<(i64, PathBuf)> = Vec::new();
    for path in session_files(directory) {
        let Some(created_at) = created_at_unix_secs(&path) else {
            continue;
        };
        let distance = (created_at - started_at).abs();
        if distance <= START_MATCH_TOLERANCE_SECS {
            candidates.push((distance, path));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    candidates.into_iter().map(|(_, path)| path).collect()
}

fn newest_session_file(directory: &Path) -> Option<PathBuf> {
    session_files(directory).into_iter().max_by_key(|path| {
        path.metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    })
}

fn created_at_unix_secs(path: &Path) -> Option<i64> {
    let stamp = path.file_name()?.to_str()?.split('_').next()?;
    let (date, time) = stamp.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.strip_suffix('Z')?.split('-');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next()?.parse().ok()?;
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn process_start_unix_secs(pid: i32) -> Option<i64> {
    let output = Command::new("ps")
        .args(["-o", "etime=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let elapsed = parse_elapsed(String::from_utf8(output.stdout).ok()?.trim())?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(now).ok().map(|now| now - elapsed)
}

fn parse_elapsed(text: &str) -> Option<i64> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.trim().parse::<i64>().ok()?, clock),
        None => (0, text),
    };
    let mut parts = clock.rsplit(':');
    let seconds: i64 = parts.next()?.trim().parse().ok()?;
    let minutes: i64 = parts.next()?.trim().parse().ok()?;
    let hours: i64 = match parts.next() {
        Some(hours) => hours.trim().parse().ok()?,
        None => 0,
    };
    Some(days * 86_400 + hours * 3_600 + minutes * 60 + seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    static SESSION_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn every_transcript_started_in_the_match_window_is_a_candidate() {
        let root = tempfile::TempDir::new().unwrap();
        let directory = root.path();
        for name in [
            "2026-08-21T10-49-27-004Z_01a023f0-9edc-7376-a318-d88ca8968107.jsonl",
            "2026-08-21T10-49-27-998Z_01a023f0-a2be-76d4-ba2a-ec473035d7db.jsonl",
        ] {
            std::fs::write(directory.join(name), b"{}\n").unwrap();
        }
        let started_at = created_at_unix_secs(
            &directory.join("2026-08-21T10-49-27-004Z_01a023f0-9edc-7376-a318-d88ca8968107.jsonl"),
        )
        .unwrap();

        let mut candidates: Vec<String> = session_files_started_at(directory, started_at)
            .into_iter()
            .filter_map(|path| id_from_path(&path))
            .collect();
        candidates.sort();

        assert_eq!(
            candidates,
            vec![
                "01a023f0-9edc-7376-a318-d88ca8968107".to_owned(),
                "01a023f0-a2be-76d4-ba2a-ec473035d7db".to_owned(),
            ],
            "two lanes started in the same second must both stay candidates"
        );
    }

    #[test]
    fn session_dir_name_matches_pis_encoding() {
        assert_eq!(
            session_dir_name("/media/kmrh47/WD_SN850X/Git/qol-monorepo"),
            "--media-kmrh47-WD_SN850X-Git-qol-monorepo--"
        );
        assert_eq!(session_dir_name("/"), "----");
    }

    #[test]
    fn session_dir_override_replaces_the_agent_dir_sessions_default() {
        let _env_guard = SESSION_DIR_ENV_LOCK.lock().unwrap();
        let previous_home = std::env::var_os("HOME");
        let previous_override = std::env::var_os("PI_CODING_AGENT_SESSION_DIR");
        std::env::set_var("HOME", "/home/u");
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", "~/relay-sessions");
        let directory = session_dir("/work/proj");
        match previous_override {
            Some(value) => std::env::set_var("PI_CODING_AGENT_SESSION_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_SESSION_DIR"),
        }
        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        assert_eq!(
            directory,
            Some(PathBuf::from("/home/u/relay-sessions/--work-proj--"))
        );
    }

    fn with_session_dir_override<T>(directory: &std::path::Path, body: impl FnOnce() -> T) -> T {
        let _env_guard = SESSION_DIR_ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os("PI_CODING_AGENT_SESSION_DIR");
        std::env::set_var("PI_CODING_AGENT_SESSION_DIR", directory);
        let result = body();
        match previous {
            Some(value) => std::env::set_var("PI_CODING_AGENT_SESSION_DIR", value),
            None => std::env::remove_var("PI_CODING_AGENT_SESSION_DIR"),
        }
        result
    }

    fn stamped_session_name(offset_secs: i64) -> String {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + offset_secs;
        let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
        let seconds_of_day = seconds.rem_euclid(86_400);
        let stamp = format!(
            "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}-000Z",
            seconds_of_day / 3600,
            (seconds_of_day % 3600) / 60,
            seconds_of_day % 60
        );
        format!("{stamp}_019fc79d-b608-7dd5-83c0-af0e4691150a.jsonl")
    }

    fn civil_from_days(days: i64) -> (i64, i64, i64) {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
        let month = month_prime + if month_prime < 10 { 3 } else { -9 };
        (year + i64::from(month <= 2), month, day)
    }

    fn marker_fixture() -> (tempfile::TempDir, PathBuf, String, SystemTime) {
        let root = tempfile::TempDir::new().unwrap();
        let directory = root.path().join("--work-proj--");
        std::fs::create_dir_all(&directory).unwrap();
        let marker = "QOL_BRIDGE_DONE_marker-test".to_owned();
        let since = SystemTime::now() - std::time::Duration::from_secs(600);
        (root, directory, marker, since)
    }

    fn write_marker_file(directory: &std::path::Path, name: &str, marker: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, marker.as_bytes()).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        file.set_modified(SystemTime::now()).unwrap();
        path
    }

    #[test]
    fn a_marker_file_with_a_fresh_mtime_is_found() {
        let (root, directory, marker, since) = marker_fixture();
        let path = write_marker_file(&directory, &stamped_session_name(-400), &marker);
        with_session_dir_override(root.path(), || {
            assert_eq!(
                session_file_containing_marker("/work/proj", &marker, since),
                Some(path),
                "a marker file modified after `since` must be found"
            );
        });
    }

    #[test]
    fn a_marker_file_with_an_old_mtime_is_excluded() {
        let (root, directory, marker, since) = marker_fixture();
        let path = write_marker_file(&directory, &stamped_session_name(-400), &marker);
        let file = std::fs::File::open(&path).unwrap();
        file.set_modified(SystemTime::now() - std::time::Duration::from_secs(7200))
            .unwrap();
        with_session_dir_override(root.path(), || {
            assert_eq!(
                session_file_containing_marker("/work/proj", &marker, since),
                None,
                "a marker file last modified before `since` must be excluded"
            );
        });
    }

    #[test]
    fn an_unparseable_name_marker_file_with_a_fresh_mtime_is_included() {
        let (root, directory, marker, since) = marker_fixture();
        assert_eq!(
            created_at_unix_secs(&directory.join("nonsense.jsonl")),
            None
        );
        let path = write_marker_file(&directory, "nonsense.jsonl", &marker);
        with_session_dir_override(root.path(), || {
            assert_eq!(
                session_file_containing_marker("/work/proj", &marker, since),
                Some(path),
                "the mtime bound alone decides inclusion, so an unparseable-name file stays a candidate"
            );
        });
    }

    #[test]
    fn the_marker_candidate_created_closest_to_since_wins_the_tiebreak() {
        let (root, directory, marker, since) = marker_fixture();
        let near = write_marker_file(&directory, &stamped_session_name(-400), &marker);
        let far = write_marker_file(&directory, &stamped_session_name(-100), &marker);
        let near_created = created_at_unix_secs(&near).unwrap();
        let far_created = created_at_unix_secs(&far).unwrap();
        let since_secs = since.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        assert!(
            (near_created - since_secs).abs() < (far_created - since_secs).abs(),
            "the near file must be created closer to `since` than the far file"
        );
        with_session_dir_override(root.path(), || {
            assert_eq!(
                session_file_containing_marker("/work/proj", &marker, since),
                Some(near),
                "among two marker-bearing files, the one created closest to `since` must win"
            );
        });
    }

    #[test]
    fn created_at_reads_the_utc_stamp_in_the_file_name() {
        let cases = [
            (
                "2026-08-03T12-33-49-576Z_019fc79d-b608-7dd5-83c0-af0e4691150a.jsonl",
                1_785_760_429,
            ),
            (
                "2026-08-03T12-37-45-895Z_019fc7a1-5126-7ae6-8749-0d2c7688c6ad.jsonl",
                1_785_760_665,
            ),
            ("1970-01-01T00-00-00-000Z_x.jsonl", 0),
        ];
        for (name, expected) in cases {
            assert_eq!(
                created_at_unix_secs(Path::new(name)),
                Some(expected),
                "name: {name}"
            );
        }
        for name in ["nonsense.jsonl", "2026-08-03_x.jsonl", "T-x.jsonl"] {
            assert_eq!(created_at_unix_secs(Path::new(name)), None, "name: {name}");
        }
    }

    #[test]
    fn parse_elapsed_covers_every_ps_shape() {
        let cases = [
            ("05", None),
            ("00:12", Some(12)),
            ("41:03", Some(2_463)),
            ("02:41:03", Some(9_663)),
            ("3-02:41:03", Some(268_863)),
            ("  1-00:00:01  ", Some(86_401)),
            ("", None),
            ("abc", None),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_elapsed(text.trim()), expected, "etime: {text:?}");
        }
    }

    #[test]
    fn each_pane_claims_the_session_started_with_it() {
        let root = tempfile::TempDir::new().unwrap();
        let first = root
            .path()
            .join("2026-08-03T12-33-49-576Z_019fc79d-b608-7dd5-83c0-af0e4691150a.jsonl");
        let second = root
            .path()
            .join("2026-08-03T12-37-45-895Z_019fc7a1-5126-7ae6-8749-0d2c7688c6ad.jsonl");
        std::fs::write(&first, "").unwrap();
        std::fs::write(&second, "").unwrap();

        assert_eq!(
            session_file_started_at(root.path(), 1_785_760_428),
            Some(first),
            "a pane started at 12:33:48 must claim the 12:33:49 session"
        );
        assert_eq!(
            session_file_started_at(root.path(), 1_785_760_665),
            Some(second),
            "a pane started at 12:37:45 must claim the 12:37:45 session"
        );
        assert_eq!(
            session_file_started_at(root.path(), 1_785_700_000),
            None,
            "a pane older than every session claims none of them"
        );
    }

    #[test]
    fn same_second_spawns_resolve_by_the_process_env_not_the_timestamp() {
        let root = tempfile::TempDir::new().unwrap();
        let target = root
            .path()
            .join("2026-08-24T19-22-22-314Z_01a03539-4aea-7cfa-98f8-16378157ce45.jsonl");
        let other = root
            .path()
            .join("2026-08-24T19-22-27-530Z_01a03539-5f4a-70fb-a215-36e00f9f3323.jsonl");
        std::fs::write(&target, "").unwrap();
        std::fs::write(&other, "").unwrap();
        let read_env = |pid: i32| (pid == 1001).then(|| target.to_string_lossy().into_owned());
        let no_children = |_: i32| Vec::new();
        assert_eq!(
            session_file_from_pi_env_with(1001, root.path(), &read_env, &no_children),
            Some(target.clone()),
            "the env-declared session file wins over any timestamp heuristic"
        );
        assert_eq!(
            session_file_from_pi_env_with(2002, root.path(), &read_env, &no_children),
            None,
            "a pid without the env falls through"
        );
    }

    #[test]
    fn the_pi_process_is_found_below_a_wrapper_without_the_env() {
        let root = tempfile::TempDir::new().unwrap();
        let file = root
            .path()
            .join("2026-08-24T19-22-27-530Z_01a03539-5f4a-70fb-a215-36e00f9f3323.jsonl");
        std::fs::write(&file, "").unwrap();
        let read_env = |pid: i32| match pid {
            10 => None,
            11 => Some(file.to_string_lossy().into_owned()),
            _ => None,
        };
        let children = |pid: i32| match pid {
            10 => vec![11],
            _ => Vec::new(),
        };
        assert_eq!(
            session_file_from_pi_env_with(10, root.path(), &read_env, &children),
            Some(file.clone()),
            "a wrapper without the env resolves through its pi child"
        );
    }

    #[test]
    fn an_env_file_outside_the_session_directory_is_rejected() {
        let root = tempfile::TempDir::new().unwrap();
        let outside_dir = root.path().join("elsewhere");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let outside = outside_dir.join("elsewhere.jsonl");
        std::fs::write(&outside, "").unwrap();
        let read_env = |pid: i32| (pid == 7).then(|| outside.to_string_lossy().into_owned());
        let no_children = |_: i32| Vec::new();
        assert_eq!(
            session_file_from_pi_env_with(7, root.path(), &read_env, &no_children),
            None,
            "a session file outside the expected directory must not be claimed"
        );
    }
}
