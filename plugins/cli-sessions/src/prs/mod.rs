use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Result};
use qol_process::BoundedCommandOutput;
use qol_sessions_engine::attention::{AttentionState, LanePr, PrChecks, PrState};
use qol_terminal_sessions::park::ParkRecord;
use serde::Deserialize;

use crate::park::store::ParkStore;
use crate::session::registry::Registry;

pub const PRS: &str = "prs";
const REFRESH: Duration = Duration::from_secs(60);
const GH_TIMEOUT: Duration = Duration::from_secs(20);
const GH_OUTPUT_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub url: String,
    pub number: u64,
    pub key: Option<String>,
    pub title: Option<String>,
    pub park: String,
    pub watchers: Vec<String>,
}

impl Opened {
    fn who(&self) -> String {
        who(self.key.as_deref(), self.title.as_deref(), &self.park)
    }
}

fn who(key: Option<&str>, title: Option<&str>, park: &str) -> String {
    key.or(title).unwrap_or(park).to_owned()
}

pub fn pr_url(arg: &str) -> Option<(String, u64)> {
    let mut parts = arg.strip_prefix("https://github.com/")?.split('/');
    let (owner, repo, pull, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || owner.is_empty() || repo.is_empty() || pull != "pull" {
        return None;
    }
    let number = number.parse().ok()?;
    Some((
        format!("https://github.com/{owner}/{repo}/pull/{number}"),
        number,
    ))
}

pub fn opened(mut parks: Vec<(ParkRecord, Option<String>)>) -> Vec<Opened> {
    parks.sort_by(|(left, _), (right, _)| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut out: Vec<Opened> = Vec::new();
    for (record, key) in parks {
        let Some((url, number)) = record.command.iter().find_map(|arg| pr_url(arg)) else {
            continue;
        };
        match out.iter_mut().find(|pr| pr.url == url) {
            Some(pr) => {
                let watcher = who(key.as_deref(), record.title.as_deref(), &record.id);
                if watcher != pr.who() && !pr.watchers.contains(&watcher) {
                    pr.watchers.push(watcher);
                }
            }
            None => out.push(Opened {
                url,
                number,
                key,
                title: record.title,
                park: record.id,
                watchers: Vec::new(),
            }),
        }
    }
    out
}

fn from_store(store: &ParkStore) -> Result<Vec<Opened>> {
    let parks = store
        .list()?
        .into_iter()
        .map(|record| {
            let key = store
                .spawn_key(&record.id)
                .map(|key| key.as_str().to_owned());
            (record, key)
        })
        .collect();
    Ok(opened(parks))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct View {
    state: String,
    is_draft: bool,
    #[serde(default)]
    status_check_rollup: Vec<Check>,
}

#[derive(Deserialize)]
struct Check {
    #[serde(default)]
    status: String,
    #[serde(default)]
    conclusion: String,
    #[serde(default)]
    state: String,
}

fn checks(rollup: &[Check]) -> PrChecks {
    let mut pending = false;
    let mut passed = false;
    for check in rollup {
        let verdict = if !check.state.is_empty() {
            check.state.as_str()
        } else if check.status != "COMPLETED" {
            "PENDING"
        } else {
            check.conclusion.as_str()
        };
        match verdict {
            "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => {
                return PrChecks::Failing
            }
            "PENDING" | "EXPECTED" => pending = true,
            "SUCCESS" => passed = true,
            _ => {}
        }
    }
    if pending {
        PrChecks::Pending
    } else if passed {
        PrChecks::Passing
    } else {
        PrChecks::None
    }
}

pub fn parse_view(url: &str, number: u64, json: &str) -> Option<LanePr> {
    let view: View = serde_json::from_str(json).ok()?;
    let state = match view.state.as_str() {
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => return None,
    };
    Some(LanePr {
        url: url.to_owned(),
        number,
        state,
        draft: view.is_draft,
        checks: checks(&view.status_check_rollup),
    })
}

fn fetch(url: &str, number: u64) -> Option<LanePr> {
    let mut command = Command::new("gh");
    command.args([
        "pr",
        "view",
        url,
        "--json",
        "state,isDraft,statusCheckRollup",
    ]);
    qol_process::hide_console_window(&mut command);
    match qol_process::run_owned_with_output_timeout(command, GH_TIMEOUT, GH_OUTPUT_LIMIT) {
        Ok(BoundedCommandOutput::Completed(output)) if output.status.success() => parse_view(
            url,
            number,
            &String::from_utf8_lossy(output.stdout.as_bytes()),
        ),
        Ok(BoundedCommandOutput::Completed(output)) => {
            log::warn!(
                "[cli-sessions] gh pr view {url} failed: {}",
                String::from_utf8_lossy(output.stderr.as_bytes()).trim()
            );
            None
        }
        Ok(BoundedCommandOutput::TimedOut { .. }) => {
            log::warn!(
                "[cli-sessions] gh pr view {url} gave no answer in {}s",
                GH_TIMEOUT.as_secs()
            );
            None
        }
        Err(error) => {
            log::warn!("[cli-sessions] gh pr view {url} did not start: {error}");
            None
        }
    }
}

pub fn spawn_refresh(registry: Arc<Mutex<Registry>>) {
    let spawned = std::thread::Builder::new()
        .name("cli-sessions-prs".to_owned())
        .spawn(move || {
            let mut known = HashMap::new();
            loop {
                refresh(&registry, &mut known);
                std::thread::sleep(REFRESH);
            }
        });
    if let Err(error) = spawned {
        log::warn!("[cli-sessions] pull request refresh not started: {error}");
    }
}

fn refresh(registry: &Arc<Mutex<Registry>>, known: &mut HashMap<String, LanePr>) {
    let Ok(keys) = registry.lock().map(|reg| reg.keys()) else {
        return;
    };
    let opened = match ParkStore::system().and_then(|store| from_store(&store)) {
        Ok(opened) => opened,
        Err(error) => {
            log::warn!("[cli-sessions] parked sessions not readable for pull requests: {error:#}");
            return;
        }
    };
    let by_key = statuses(opened, &keys, known, fetch);
    if let Ok(mut reg) = registry.lock() {
        reg.set_prs(&by_key);
    }
}

pub fn statuses(
    opened: Vec<Opened>,
    keys: &[String],
    known: &mut HashMap<String, LanePr>,
    mut fetch: impl FnMut(&str, u64) -> Option<LanePr>,
) -> HashMap<String, Vec<LanePr>> {
    known.retain(|url, _| opened.iter().any(|pr| &pr.url == url));
    let mut by_key: HashMap<String, Vec<LanePr>> = HashMap::new();
    for pr in opened {
        let Some(key) = pr.key.filter(|key| keys.contains(key)) else {
            continue;
        };
        let merged = known
            .get(&pr.url)
            .is_some_and(|status| status.state == PrState::Merged);
        if !merged {
            if let Some(status) = fetch(&pr.url, pr.number) {
                known.insert(pr.url.clone(), status);
            }
        }
        if let Some(status) = known.get(&pr.url) {
            by_key.entry(key).or_default().push(status.clone());
        }
    }
    for prs in by_key.values_mut() {
        prs.sort_by_key(|pr| std::cmp::Reverse(pr.number));
    }
    by_key
}

pub fn run(args: &[String]) -> Result<String> {
    let opened = from_store(&ParkStore::system()?)?;
    match args {
        [] => Ok(list(&opened)),
        [target] => find(&opened, target),
        _ => bail!("expected at most one pull request url or number"),
    }
}

fn list(opened: &[Opened]) -> String {
    let mut rows = opened.iter().collect::<Vec<_>>();
    rows.sort_by_key(|pr| std::cmp::Reverse(pr.number));
    rows.into_iter()
        .map(|pr| format!("#{}\t{}\t{}", pr.number, pr.who(), pr.url))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn matching<'a>(opened: &'a [Opened], target: &str) -> Vec<&'a Opened> {
    let number = target.trim_start_matches('#').parse::<u64>().ok();
    let url = pr_url(target).map(|(url, _)| url);
    opened
        .iter()
        .filter(|pr| Some(pr.number) == number || url.as_deref() == Some(pr.url.as_str()))
        .collect()
}

fn find(opened: &[Opened], target: &str) -> Result<String> {
    let found = matching(opened, target);
    if found.is_empty() {
        bail!("no session parked on `{target}`; a pull request is tied to the session that first parked on its watcher");
    }
    let attention = qol_sessions_engine::attention::read();
    Ok(found
        .into_iter()
        .map(|pr| {
            let live = pr.key.as_deref().and_then(|key| {
                attention
                    .as_ref()?
                    .sessions
                    .iter()
                    .find_map(|row| (row.key.as_deref() == Some(key)).then_some(row.state))
            });
            let mut lines = vec![
                pr.url.clone(),
                format!("opened by: {} (park {})", pr.who(), pr.park),
                format!("session: {}", live.map_or("closed", attention_label)),
            ];
            if let Some(status) = fetch(&pr.url, pr.number) {
                lines.push(format!("pull request: {}", status_label(&status)));
            }
            if !pr.watchers.is_empty() {
                lines.push(format!("also watched by: {}", pr.watchers.join(", ")));
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n"))
}

fn attention_label(state: AttentionState) -> &'static str {
    match state {
        AttentionState::NeedsYou => "needs you",
        AttentionState::YourTurn => "your turn",
        AttentionState::Working => "working",
        AttentionState::Idle => "idle",
        AttentionState::Parked => "parked",
        AttentionState::Ready => "ready",
    }
}

pub fn status_label(pr: &LanePr) -> String {
    let state = match pr.state {
        PrState::Open if pr.draft => "draft",
        PrState::Open => "ready",
        PrState::Merged => "merged",
        PrState::Closed => "closed",
    };
    let checks = match pr.checks {
        PrChecks::Pending => "checks pending",
        PrChecks::Passing => "checks passing",
        PrChecks::Failing => "checks failing",
        PrChecks::None => "no checks",
    };
    format!("{state}, {checks}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::park::ParkState;

    fn park(id: &str, created_at: u64, url: &str, title: &str) -> ParkRecord {
        ParkRecord {
            id: id.to_owned(),
            created_at,
            title: Some(title.to_owned()),
            command: vec![
                "node".to_owned(),
                "pr-watch.cjs".to_owned(),
                url.to_owned(),
                "--pretty".to_owned(),
            ],
            ..crate::park::tests::record(ParkState::Finished)
        }
    }

    #[test]
    fn a_pull_request_url_is_read_only_from_a_whole_github_pull_link() {
        assert_eq!(
            pr_url("https://github.com/qol-tools/qol/pull/187"),
            Some(("https://github.com/qol-tools/qol/pull/187".to_owned(), 187))
        );
        for other in [
            "https://github.com/qol-tools/qol/pull/187/files",
            "https://github.com/qol-tools/qol/issues/187",
            "https://github.com/qol-tools/qol/pull/x",
            "github.com/qol-tools/qol/pull/187",
            "--pretty",
        ] {
            assert_eq!(pr_url(other), None, "{other}");
        }
    }

    #[test]
    fn the_first_session_to_park_on_a_pull_request_opened_it() {
        let url = "https://github.com/qol-tools/qol/pull/187";
        let parks = vec![
            (
                park("park-3-3", 3, url, "fix"),
                Some("clipboard-sec-fix".to_owned()),
            ),
            (
                park("park-1-1", 1, url, "push"),
                Some("clipboard-sec-push".to_owned()),
            ),
            (
                park("park-2-2", 2, url, "push"),
                Some("clipboard-sec-push".to_owned()),
            ),
            (park("park-4-4", 4, url, "loose chat"), None),
            (
                park(
                    "park-5-5",
                    5,
                    "https://github.com/qol-tools/qol/pull/192",
                    "ui",
                ),
                Some("sessions-ui-parity".to_owned()),
            ),
            (
                ParkRecord {
                    command: vec!["cargo".to_owned(), "build".to_owned()],
                    ..park("park-6-6", 6, url, "build")
                },
                Some("builder".to_owned()),
            ),
        ];
        let opened = opened(parks);
        assert_eq!(opened.len(), 2);
        assert_eq!(opened[0].key.as_deref(), Some("clipboard-sec-push"));
        assert_eq!(opened[0].park, "park-1-1");
        assert_eq!(opened[0].watchers, ["clipboard-sec-fix", "loose chat"]);
        assert_eq!(opened[1].number, 192);
        assert_eq!(matching(&opened, "187").len(), 1);
        assert_eq!(
            matching(&opened, "#192")[0].key.as_deref(),
            Some("sessions-ui-parity")
        );
        assert_eq!(matching(&opened, url)[0].number, 187);
        assert!(matching(&opened, "999").is_empty());
    }

    #[test]
    fn a_failed_fetch_keeps_the_last_status_and_only_a_merge_is_final() {
        let pr = |number: u64, key: &str| Opened {
            url: format!("https://github.com/qol-tools/qol/pull/{number}"),
            number,
            key: Some(key.to_owned()),
            title: None,
            park: format!("park-{number}"),
            watchers: Vec::new(),
        };
        let status = |number: u64, state| LanePr {
            url: format!("https://github.com/qol-tools/qol/pull/{number}"),
            number,
            state,
            draft: false,
            checks: PrChecks::Passing,
        };
        let keys = ["lane".to_owned()];
        let opened = || vec![pr(1, "lane"), pr(2, "lane"), pr(3, "lane"), pr(4, "gone")];
        let mut known = HashMap::new();
        let mut asked = Vec::new();
        let first = statuses(opened(), &keys, &mut known, |_, number| {
            asked.push(number);
            Some(status(
                number,
                [PrState::Open, PrState::Merged, PrState::Closed][number as usize - 1],
            ))
        });
        assert_eq!(asked, [1, 2, 3], "a lane that is not live is never fetched");
        assert_eq!(first["lane"].len(), 3);

        let mut asked = Vec::new();
        let second = statuses(opened(), &keys, &mut known, |_, number| {
            asked.push(number);
            (number == 3).then(|| status(3, PrState::Open))
        });
        assert_eq!(asked, [1, 3], "a merged pull request is not fetched again");
        assert_eq!(
            second,
            first
                .clone()
                .into_iter()
                .map(|(key, mut prs)| {
                    prs[0] = status(3, PrState::Open);
                    (key, prs)
                })
                .collect(),
            "a failed fetch keeps the last status and a reopened one updates"
        );

        statuses(vec![pr(1, "lane")], &keys, &mut known, |_, _| None);
        assert_eq!(
            known.keys().collect::<Vec<_>>(),
            ["https://github.com/qol-tools/qol/pull/1"]
        );
    }

    #[test]
    fn checks_fail_on_any_failure_and_wait_on_any_pending_run() {
        let view = |rollup: &str| {
            parse_view(
                "u",
                1,
                &format!(r#"{{"state":"OPEN","isDraft":false,"statusCheckRollup":[{rollup}]}}"#),
            )
            .map(|pr| pr.checks)
        };
        let done =
            |conclusion: &str| format!(r#"{{"status":"COMPLETED","conclusion":"{conclusion}"}}"#);
        let running = r#"{"status":"IN_PROGRESS","conclusion":""}"#;
        assert_eq!(view(""), Some(PrChecks::None));
        assert_eq!(view(&done("SUCCESS")), Some(PrChecks::Passing));
        assert_eq!(
            view(&format!("{},{}", done("SUCCESS"), done("SKIPPED"))),
            Some(PrChecks::Passing)
        );
        assert_eq!(
            view(&format!("{},{running}", done("SUCCESS"))),
            Some(PrChecks::Pending)
        );
        assert_eq!(
            view(&format!("{running},{}", done("FAILURE"))),
            Some(PrChecks::Failing)
        );
        assert_eq!(view(&done("CANCELLED")), Some(PrChecks::None));
        assert_eq!(view(r#"{"state":"ERROR"}"#), Some(PrChecks::Failing));
        assert_eq!(view(r#"{"state":"PENDING"}"#), Some(PrChecks::Pending));
    }

    #[test]
    fn the_pull_request_state_and_draft_come_from_gh() {
        let merged = parse_view(
            "u",
            7,
            r#"{"state":"MERGED","isDraft":false,"statusCheckRollup":[]}"#,
        )
        .unwrap();
        assert_eq!((merged.state, merged.number), (PrState::Merged, 7));
        let draft = parse_view("u", 7, r#"{"state":"OPEN","isDraft":true}"#).unwrap();
        assert_eq!(status_label(&draft), "draft, no checks");
        assert!(parse_view("u", 7, r#"{"state":"WHAT","isDraft":true}"#).is_none());
        assert!(parse_view("u", 7, "not json").is_none());
    }
}
