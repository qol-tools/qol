use qol_conventions::pr_try::{self, ListedBuild};
use serde::Deserialize;

use super::model::{RowAction, Summary, Trial, UpdatesSnapshot};

pub(super) const TITLE: &str = "Pull request build";
pub(super) const GROUP: &str = "Development";
pub(super) const GROUP_DETAIL: &str = "Turned on in qol settings, under Testing.";
const OFF: &str = "Off";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum Job {
    Getting { number: u64 },
    Ending,
    NeedsTray { number: u64 },
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BuildRow {
    pub(super) value: String,
    pub(super) description: String,
    pub(super) spinner: bool,
    pub(super) attention: bool,
    pub(super) opens: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CardRow {
    pub(super) label: String,
    pub(super) detail: String,
    pub(super) running: bool,
    pub(super) action: Option<RowAction>,
    pub(super) verb: Option<&'static str>,
}

pub(super) fn ends_try(snapshot: &UpdatesSnapshot) -> &'static str {
    if snapshot.can_try_pull_requests {
        "Pull request build is Off"
    } else {
        "qol pr off"
    }
}

pub(super) fn checks_resume(snapshot: &UpdatesSnapshot) -> &'static str {
    if snapshot.can_try_pull_requests {
        "Back when the try ends"
    } else {
        "Back after qol pr off"
    }
}

pub(super) fn row(snapshot: &UpdatesSnapshot) -> BuildRow {
    let trying = snapshot.trial.as_ref().map(|trial| trial.pull_request);
    let current = trying.map_or_else(|| OFF.to_string(), |number| format!("#{number}"));
    let idle = |description: String| BuildRow {
        value: current.clone(),
        description,
        spinner: false,
        attention: false,
        opens: true,
    };
    match &snapshot.pull_request_build {
        Some(Job::Getting { number }) => BuildRow {
            value: format!("#{number}"),
            description: format!("Getting #{number}'s CI build."),
            spinner: true,
            attention: false,
            opens: false,
        },
        Some(Job::Ending) => BuildRow {
            value: OFF.to_string(),
            description: "Putting the installed version back.".to_string(),
            spinner: true,
            attention: false,
            opens: false,
        },
        Some(Job::NeedsTray { number }) => BuildRow {
            opens: false,
            ..idle(format!("Trying #{number} restarts qol-tray."))
        },
        Some(Job::Failed { message }) => BuildRow {
            attention: true,
            ..idle(sentence(message))
        },
        None => idle(match trying {
            Some(number) => format!("#{number}'s CI build runs in place of the installed one."),
            None => "Runs a pull request's CI build in place of the installed one.".to_string(),
        }),
    }
}

pub(super) fn busy_summary(snapshot: &UpdatesSnapshot) -> Option<Summary> {
    let (label, description) = match snapshot.pull_request_build.as_ref()? {
        Job::Getting { number } => (
            format!("Getting pull request #{number}"),
            "Downloading its CI build. The installed version runs until it is in place.",
        ),
        Job::Ending => (
            "Ending the try".to_string(),
            "Putting the installed version back, then checks turn on again.",
        ),
        Job::NeedsTray { .. } | Job::Failed { .. } => return None,
    };
    Some(Summary {
        dot: None,
        busy: true,
        label,
        description: description.to_string(),
        action: None,
        action_label: None,
    })
}

pub(super) fn needs_tray(snapshot: &UpdatesSnapshot) -> Option<u64> {
    match snapshot.pull_request_build {
        Some(Job::NeedsTray { number }) => Some(number),
        _ => None,
    }
}

pub(super) fn tray_question(number: u64) -> String {
    format!("Trying #{number} restarts qol-tray, and the switchboard closes.")
}

pub(super) fn card_rows(builds: &[ListedBuild], trial: Option<&Trial>) -> Vec<CardRow> {
    let trying = trial.map(|trial| trial.pull_request);
    let runs = |build: &ListedBuild| {
        trial.is_some_and(|trial| {
            trial.pull_request == build.number
                && pr_try::short_sha(&trial.head) == pr_try::short_sha(&build.head)
        })
    };
    let mut rows = vec![CardRow {
        label: OFF.to_string(),
        detail: "The installed version runs.".to_string(),
        running: trying.is_none(),
        action: trying.map(|_| RowAction::EndTry),
        verb: trying.map(|_| "choose"),
    }];
    if let Some(trial) = trial.filter(|_| !builds.iter().any(runs)) {
        rows.push(CardRow {
            label: format!("#{}", trial.pull_request),
            detail: built_from(&trial.head),
            running: true,
            action: None,
            verb: None,
        });
    }
    rows.extend(builds.iter().map(|build| {
        let running = runs(build);
        CardRow {
            label: format!("#{} {}", build.number, build.title),
            detail: built_from(&build.head),
            running,
            action: (!running).then_some(RowAction::TryPullRequest(build.number)),
            verb: (!running).then_some("try"),
        }
    }));
    rows
}

pub(super) fn action_request(action: &RowAction) -> Option<(&'static str, Option<String>)> {
    let with_number =
        |number: &u64| Some(serde_json::json!({ "id": number.to_string() }).to_string());
    Some(match action {
        RowAction::TryPullRequest(number) => (pr_try::TRY_ACTION, with_number(number)),
        RowAction::TryWithTray(number) => (pr_try::TRY_WITH_TRAY_ACTION, with_number(number)),
        RowAction::EndTry => (pr_try::END_ACTION, None),
        RowAction::KeepTray => (pr_try::DISMISS_ACTION, None),
        RowAction::Check | RowAction::Update(_) | RowAction::UpdateAll | RowAction::StopUpdates => {
            return None
        }
    })
}

fn built_from(head: &str) -> String {
    if head.is_empty() {
        String::new()
    } else {
        format!("Built from {head}")
    }
}

fn sentence(message: &str) -> String {
    let message = message.trim();
    if message.ends_with('.') {
        message.to_string()
    } else {
        format!("{message}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(number: u64) -> ListedBuild {
        ListedBuild {
            number,
            title: format!("Change {number}"),
            head: "d3b2832".to_string(),
        }
    }

    fn trial(number: u64) -> Trial {
        Trial {
            pull_request: number,
            head: "d3b2832".to_string(),
        }
    }

    #[test]
    fn off_runs_until_a_pull_request_is_tried() {
        let rows = card_rows(&[build(621), build(620)], None);
        let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["Off", "#621 Change 621", "#620 Change 620"]);
        assert!(rows[0].running);
        assert_eq!(rows[0].action, None);
        assert_eq!(rows[1].action, Some(RowAction::TryPullRequest(621)));
        assert_eq!(rows[1].verb, Some("try"));
        assert_eq!(rows[1].detail, "Built from d3b2832");
    }

    #[test]
    fn while_trying_the_running_build_is_tagged_and_off_ends_the_try() {
        let tried = trial(620);
        let rows = card_rows(&[build(621), build(620)], Some(&tried));
        assert!(!rows[0].running);
        assert_eq!(rows[0].action, Some(RowAction::EndTry));
        assert_eq!(rows[0].verb, Some("choose"));
        assert!(rows[2].running);
        assert_eq!(rows[2].action, None);
        assert_eq!(rows[1].action, Some(RowAction::TryPullRequest(621)));
    }

    #[test]
    fn a_newer_build_of_the_running_pull_request_stays_tryable() {
        let tried = Trial {
            pull_request: 620,
            head: "0aa1bb2cc3".to_string(),
        };
        let rows = card_rows(&[build(621), build(620)], Some(&tried));
        let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Off", "#620", "#621 Change 621", "#620 Change 620"]
        );
        assert!(rows[1].running);
        assert_eq!(rows[1].detail, "Built from 0aa1bb2cc3");
        assert!(!rows[3].running);
        assert_eq!(rows[3].action, Some(RowAction::TryPullRequest(620)));
    }

    #[test]
    fn a_running_build_that_left_the_list_still_shows() {
        let tried = trial(600);
        let rows = card_rows(&[build(621)], Some(&tried));
        let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["Off", "#600", "#621 Change 621"]);
        assert!(rows[1].running);
    }

    #[test]
    fn actions_name_the_tray_routes() {
        let numbered = Some(r#"{"id":"621"}"#.to_string());
        let cases = [
            (
                RowAction::TryPullRequest(621),
                Some(("try_pull_request", numbered.clone())),
            ),
            (
                RowAction::TryWithTray(621),
                Some(("try_pull_request_with_tray", numbered)),
            ),
            (RowAction::EndTry, Some(("end_pull_request_try", None))),
            (
                RowAction::KeepTray,
                Some(("dismiss_pull_request_try", None)),
            ),
            (RowAction::Check, None),
        ];
        for (action, expected) in cases {
            assert_eq!(action_request(&action), expected, "{action:?}");
        }
    }

    #[test]
    fn jobs_parse_from_the_tray() {
        let parse = |json: &str| serde_json::from_str::<Job>(json).unwrap();
        assert_eq!(
            parse(r#"{"state":"getting","number":621}"#),
            Job::Getting { number: 621 }
        );
        assert_eq!(parse(r#"{"state":"ending"}"#), Job::Ending);
        assert_eq!(
            parse(r#"{"state":"needs_tray","number":621}"#),
            Job::NeedsTray { number: 621 }
        );
        assert_eq!(
            parse(r#"{"state":"failed","message":"gh failed"}"#),
            Job::Failed {
                message: "gh failed".to_string()
            }
        );
    }

    #[test]
    fn the_tray_question_names_the_pull_request() {
        assert_eq!(
            tray_question(621),
            "Trying #621 restarts qol-tray, and the switchboard closes."
        );
    }

    #[test]
    fn failures_read_as_sentences() {
        assert_eq!(sentence("gh failed "), "gh failed.");
        assert_eq!(sentence("Done."), "Done.");
    }
}
