use std::fs::File;
use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_process::OwnedProcessTree;
use serde_json::Value;

use super::service::Verifier;
use super::{profile, Fact, Prediction};

const RESPONSE_LIMIT: u64 = 128 * 1024;

pub struct Ollama {
    endpoint: Option<String>,
    root: PathBuf,
    owned: Option<(Child, OwnedProcessTree)>,
    client: ureq::Agent,
    identity: String,
}

impl Ollama {
    pub fn new(root: PathBuf, endpoint: &str) -> Result<Self> {
        let endpoint = match endpoint.trim() {
            "" => None,
            value => Some(local_endpoint(value)?),
        };
        Ok(Self {
            identity: format!(
                "{}:{}",
                profile().digest,
                endpoint.as_deref().unwrap_or("owned")
            ),
            endpoint,
            root,
            owned: None,
            client: ureq::AgentBuilder::new()
                .try_proxy_from_env(false)
                .redirects(0)
                .timeout(Duration::from_secs(45))
                .timeout_connect(Duration::from_secs(2))
                .build(),
        })
    }

    fn endpoint(&mut self) -> Result<String> {
        if let Some((child, _)) = self.owned.as_mut() {
            if child.try_wait()?.is_some() {
                self.owned = None;
                self.endpoint = None;
            }
        }
        if let Some(endpoint) = &self.endpoint {
            return Ok(endpoint.clone());
        }
        let reservation = TcpListener::bind(("127.0.0.1", 0))?;
        let address = reservation.local_addr()?;
        let endpoint = format!("http://{address}");
        std::fs::create_dir_all(&self.root)?;
        let log = File::create(self.root.join("verifier-provider.log"))?;
        let mut command = Command::new("ollama");
        command
            .arg("serve")
            .env("OLLAMA_HOST", address.to_string())
            .env("OLLAMA_NO_CLOUD", "1")
            .env("OLLAMA_NUM_PARALLEL", "1")
            .env("OLLAMA_MAX_LOADED_MODELS", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        drop(reservation);
        self.owned = Some(
            qol_process::spawn_owned(command).context("local Ollama executable is unavailable")?,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some((child, _)) = self.owned.as_mut() {
                if child.try_wait()?.is_some() {
                    bail!("local Ollama server exited during startup");
                }
            }
            if self.api(&endpoint, "version", None).is_ok() {
                break;
            }
            if Instant::now() >= deadline {
                bail!("local Ollama server did not become ready");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        self.endpoint = Some(endpoint.clone());
        Ok(endpoint)
    }

    fn api(&self, endpoint: &str, route: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{endpoint}/api/{route}");
        let response = match body {
            Some(body) => self.client.post(&url).send_json(body)?,
            None => self.client.get(&url).call()?,
        };
        let mut raw = Vec::new();
        response
            .into_reader()
            .take(RESPONSE_LIMIT + 1)
            .read_to_end(&mut raw)?;
        if raw.len() as u64 > RESPONSE_LIMIT {
            bail!("model response exceeds limit");
        }
        serde_json::from_slice(&raw).context("invalid local model response")
    }

    fn verify_identity(&self, endpoint: &str) -> Result<()> {
        let registry = self.api(endpoint, "tags", None)?;
        let model = registry["models"]
            .as_array()
            .and_then(|models| models.iter().find(|model| model["name"] == profile().model))
            .context("the evaluated local verification model is not installed")?;
        if model["digest"] != profile().digest
            || model.get("remote_host").is_some()
            || model.get("remote_model").is_some()
        {
            bail!("local model does not match the evaluated verification profile");
        }
        Ok(())
    }

    fn predict(&self, endpoint: &str, query: &str, facts: &[Fact]) -> Result<Prediction> {
        let request = super::request(&profile().model, query, facts);
        if request["prompt"].as_str().map_or(usize::MAX, str::len) > profile().context_byte_limit {
            bail!("verification evidence exceeds the model context budget");
        }
        let response = self.api(endpoint, "generate", Some(request))?;
        if response["done"] != true || response["done_reason"] != "stop" {
            bail!("local verification did not produce a complete response");
        }
        let raw = response["response"]
            .as_str()
            .context("local verification returned no answer")?;
        serde_json::from_str(raw).context("local verification returned invalid answer data")
    }
}

impl Verifier for Ollama {
    fn identity(&self) -> &str {
        &self.identity
    }

    fn verify(&mut self, query: &str, facts: &[Fact]) -> Result<Prediction> {
        let endpoint = self.endpoint()?;
        self.verify_identity(&endpoint)?;
        let mut prediction = self.predict(&endpoint, query, facts)?;
        if matches!(
            super::check(query, facts, &prediction),
            super::Decision::Accepted(_)
        ) {
            let mut reversed = facts.to_vec();
            reversed.reverse();
            let confirmation = self.predict(&endpoint, query, &reversed)?;
            if super::check(query, facts, &prediction) != super::check(query, facts, &confirmation)
            {
                prediction.answers.clear();
                qol_runtime::probe!("QOL_MEMORY_DAEMON", "event=verification_order_disagreement");
            }
        }
        self.verify_identity(&endpoint)?;
        Ok(prediction)
    }
}

impl Drop for Ollama {
    fn drop(&mut self) {
        if let Some((mut child, owned)) = self.owned.take() {
            let _ = owned.terminate_and_wait(&mut child, Duration::from_secs(2));
        }
    }
}

fn local_endpoint(value: &str) -> Result<String> {
    let address = value
        .strip_prefix("http://")
        .context("verification endpoint must use local HTTP")?
        .trim_end_matches('/')
        .parse::<SocketAddr>()
        .context("verification endpoint must name a loopback IP and port")?;
    if !address.ip().is_loopback() || address.port() == 0 {
        bail!("verification endpoint must be loopback");
    }
    Ok(format!("http://{address}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_never_redirects_private_memories_to_remote_hosts() {
        for input in [
            "https://127.0.0.1:1234",
            "http://example.com:1234",
            "http://192.168.1.2:1234",
            "http://user@127.0.0.1:1234",
            "http://127.0.0.1:1234/api",
            "http://127.0.0.1:1234?redirect=remote",
        ] {
            assert!(local_endpoint(input).is_err(), "{input}");
        }
        assert_eq!(
            local_endpoint("http://127.0.0.1:11434/").unwrap(),
            "http://127.0.0.1:11434"
        );
        assert_eq!(
            local_endpoint("http://[::1]:11434").unwrap(),
            "http://[::1]:11434"
        );
    }
}
