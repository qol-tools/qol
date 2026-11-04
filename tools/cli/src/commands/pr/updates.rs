use anyhow::{bail, Context, Result};
use qol_runtime::local_http::{Client, Method};
use serde::Deserialize;
use std::time::{Duration, Instant};

const QUERY: &str = "/api/core/queries/updates";
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_WAIT: Duration = Duration::from_secs(90);
const UPDATE_WAIT: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_secs(1);
const AVAILABLE: &str = "available";

#[derive(Debug, Deserialize)]
struct Snapshot {
    host: Row,
    plugins: Vec<Row>,
    checking: bool,
    running: bool,
}

#[derive(Debug, Deserialize)]
struct Row {
    id: String,
    current: Option<String>,
    latest: Option<String>,
    state: String,
}

#[derive(Debug, Deserialize)]
struct ActionResponse {
    success: bool,
    message: String,
}

pub(super) fn catch_up(put_back: &[String]) {
    if put_back.is_empty() {
        return;
    }
    if let Err(error) = run(put_back) {
        println!("  updates: {error:#}; the tray's Updates page installs them");
    }
}

fn run(put_back: &[String]) -> Result<()> {
    let client =
        Client::tray(qol_plugin_api::host_exec::read_auth_token()?).with_io_timeout(IO_TIMEOUT);
    action(&client, "check_updates", None)?;
    let checked = wait(&client, CHECK_WAIT, |snapshot| !snapshot.checking)?;
    let (host, plugins) = stale(&checked, put_back);
    for row in &plugins {
        println!(
            "  {}: updating {} -> {}",
            row.id,
            version(&row.current),
            version(&row.latest)
        );
        action(&client, "update", Some(&row.id))?;
        wait(&client, UPDATE_WAIT, |snapshot| !snapshot.running)?;
    }
    if let Some(row) = host {
        println!(
            "  {}: updating {} -> {}",
            row.id,
            version(&row.current),
            version(&row.latest)
        );
        action(&client, "update", Some(&row.id))?;
    }
    if plugins.is_empty() && host.is_none() {
        println!("  updates: what was put back is up to date");
    }
    Ok(())
}

fn stale<'a>(snapshot: &'a Snapshot, put_back: &[String]) -> (Option<&'a Row>, Vec<&'a Row>) {
    let wanted = |row: &&Row| row.state == AVAILABLE && put_back.contains(&row.id);
    let host = Some(&snapshot.host).filter(wanted);
    let plugins = snapshot.plugins.iter().filter(wanted).collect();
    (host, plugins)
}

fn version(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("?")
}

fn wait(client: &Client, limit: Duration, done: impl Fn(&Snapshot) -> bool) -> Result<Snapshot> {
    let deadline = Instant::now() + limit;
    loop {
        let snapshot = query(client)?;
        if done(&snapshot) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            bail!("the tray did not finish within {}s", limit.as_secs());
        }
        std::thread::sleep(POLL);
    }
}

fn query(client: &Client) -> Result<Snapshot> {
    let response = client
        .request(Method::Get, QUERY, None)
        .context("asking the tray for updates")?;
    serde_json::from_str(&response.body).context("the tray's updates answer is unreadable")
}

fn action(client: &Client, name: &str, id: Option<&str>) -> Result<()> {
    let body = id.map(|id| serde_json::json!({ "id": id }).to_string());
    let route = format!("/api/core/actions/{name}");
    let response = client
        .request(Method::Post, &route, body.as_deref())
        .with_context(|| format!("asking the tray to {name}"))?;
    let answer: ActionResponse = serde_json::from_str(&response.body)
        .with_context(|| format!("the tray's {name} answer is unreadable"))?;
    if !answer.success {
        bail!("the tray refused {name}: {}", answer.message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, state: &str) -> Row {
        Row {
            id: id.to_string(),
            current: Some("1.0.0".to_string()),
            latest: Some("1.1.0".to_string()),
            state: state.to_string(),
        }
    }

    #[test]
    fn only_what_off_put_back_and_has_an_update_is_updated() {
        let snapshot = Snapshot {
            host: row("qol-tray", AVAILABLE),
            plugins: vec![
                row("qol-alt-tab", AVAILABLE),
                row("qol-launcher", "up_to_date"),
                row("qol-lights", AVAILABLE),
            ],
            checking: false,
            running: false,
        };
        let put_back = ["qol-alt-tab".to_string(), "qol-launcher".to_string()];
        let (host, plugins) = stale(&snapshot, &put_back);
        assert!(host.is_none());
        assert_eq!(
            plugins
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["qol-alt-tab"]
        );
        let with_tray = ["qol-tray".to_string()];
        assert_eq!(
            stale(&snapshot, &with_tray).0.map(|row| row.id.as_str()),
            Some("qol-tray")
        );
    }

    #[test]
    fn the_tray_snapshot_parses() {
        let body = r#"{"check_error":null,"checking":false,"running":false,"checks_enabled":true,
            "host":{"id":"qol-tray","name":"qol-tray","current":"3.136.2","latest":"3.138.0","state":"available","progress":null,"error":null,"updated_from":null},
            "plugins":[{"id":"qol-alt-tab","name":"Alt Tab","current":"0.125.0","latest":null,"state":"up_to_date","error":null}]}"#;
        let snapshot: Snapshot = serde_json::from_str(body).unwrap();
        assert_eq!(snapshot.host.state, AVAILABLE);
        assert_eq!(snapshot.plugins[0].latest, None);
    }
}
