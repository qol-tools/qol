use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::io::IsTerminal;
use std::process::Command;

use super::download::gh;

const SHA_LENGTH: usize = 40;
const FIELDS: &str = "number,headRefOid,url";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Head {
    number: u64,
    head_ref_oid: String,
    url: String,
}

pub(super) fn run(repo: &str, number: Option<u64>) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("qol pr approve opens your browser for the owner's approval, so it runs only in a terminal you type into");
    }
    let head = head(repo, number)?;
    let line = approval(&head.head_ref_oid)?;
    crate::host_facade::copy_to_clipboard(&line)?;
    open_comment_box(repo, head.number)?;
    Ok(format!(
        "copied the approval of pull request #{} at {}: paste it as a new comment on the page your browser opened",
        head.number,
        qol_conventions::pr_try::short_sha(&head.head_ref_oid)
    ))
}

fn open_comment_box(repo: &str, number: u64) -> Result<()> {
    let status = Command::new("gh")
        .args([
            "pr",
            "comment",
            &number.to_string(),
            "--repo",
            repo,
            "--web",
        ])
        .status()
        .context("could not run gh")?;
    if !status.success() {
        bail!("gh could not open pull request #{number} in your browser");
    }
    Ok(())
}

fn head(repo: &str, number: Option<u64>) -> Result<Head> {
    let view = match number {
        Some(number) => gh(&[
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            repo,
            "--json",
            FIELDS,
        ])?,
        None => gh(&["pr", "view", "--json", FIELDS])
            .context("no pull request found for this folder's branch; pass its number")?,
    };
    let head: Head =
        serde_json::from_str(&view).context("gh pr view printed something other than JSON")?;
    if !belongs(&head.url, repo) {
        bail!(
            "{} is not a pull request of {repo}; qol pr approve approves only those",
            head.url
        );
    }
    Ok(head)
}

fn belongs(url: &str, repo: &str) -> bool {
    url.strip_prefix("https://github.com/")
        .and_then(|rest| rest.strip_prefix(repo))
        .is_some_and(|rest| rest.starts_with("/pull/"))
}

fn approval(sha: &str) -> Result<String> {
    let hex = sha
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if sha.len() != SHA_LENGTH || !hex {
        bail!("`{sha}` is not a full commit id, so there is nothing to approve");
    }
    Ok(format!("/gate-approve {sha}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_approval_names_a_full_commit_id_and_nothing_else() {
        let sha = "fcfb0e9cde986e95eac2c753e9e4b7f1e1f4af33";
        assert_eq!(approval(sha).unwrap(), format!("/gate-approve {sha}"));
        for bad in [
            "",
            "fcfb0e9",
            "FCFB0E9CDE986E95EAC2C753E9E4B7F1E1F4AF33",
            "fcfb0e9cde986e95eac2c753e9e4b7f1e1f4af3g",
            "fcfb0e9cde986e95eac2c753e9e4b7f1e1f4af33 ",
        ] {
            assert!(approval(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn only_a_pull_request_of_the_repository_belongs() {
        let repo = "qol-tools/qol";
        assert!(belongs("https://github.com/qol-tools/qol/pull/500", repo));
        for other in [
            "https://github.com/qol-tools/qol-skills/pull/5",
            "https://github.com/someone/qol/pull/5",
            "https://github.com/qol-tools/qol/issues/5",
            "https://example.com/qol-tools/qol/pull/5",
        ] {
            assert!(!belongs(other, repo), "{other}");
        }
    }
}
