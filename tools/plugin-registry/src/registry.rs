use crate::oci::{sha256_digest, MANIFEST_MEDIA_TYPE};
use anyhow::{Context, Result};
use base64::Engine;
use qol_plugin_index::{BearerChallenge, RegistryLocation};
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const BODY_LIMIT: u64 = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(300);
const TAGS_PAGE: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Pull,
    PullPush,
}

pub struct Credentials {
    pub username: String,
    pub password: String,
}

pub struct Fetched {
    pub digest: String,
    pub bytes: Vec<u8>,
}

pub struct Registry {
    location: RegistryLocation,
    agent: ureq::Agent,
    access: Access,
    credentials: Option<Credentials>,
    token: Mutex<Option<String>>,
    requests: AtomicUsize,
}

struct Request<'a> {
    method: &'a str,
    url: String,
    accept: Option<&'a str>,
    content_type: Option<&'a str>,
    body: Option<&'a [u8]>,
}

impl<'a> Request<'a> {
    fn new(method: &'a str, url: String) -> Self {
        Self {
            method,
            url,
            accept: None,
            content_type: None,
            body: None,
        }
    }
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .user_agent("qol-plugin-registry")
        .build()
}

pub fn read_body(response: ureq::Response, url: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(BODY_LIMIT)
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read {url}"))?;
    Ok(bytes)
}

pub fn send(request: ureq::Request, body: Option<&[u8]>, url: &str) -> Result<ureq::Response> {
    let result = match body {
        Some(body) => request.send_bytes(body),
        None => request.call(),
    };
    match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => Ok(response),
        Err(error) => Err(error).with_context(|| format!("could not reach {url}")),
    }
}

impl Registry {
    pub fn new(reference: &str, access: Access, credentials: Option<Credentials>) -> Result<Self> {
        Ok(Self {
            location: location(reference)?,
            agent: agent(),
            access,
            credentials,
            token: Mutex::new(None),
            requests: AtomicUsize::new(0),
        })
    }

    pub fn location(&self) -> &RegistryLocation {
        &self.location
    }

    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }

    pub fn tags(&self) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct TagList {
            tags: Option<Vec<String>>,
        }
        let mut tags = Vec::new();
        let mut next = Some(self.url(&format!("tags/list?n={TAGS_PAGE}")));
        while let Some(url) = next.take() {
            let response = self.call(Request::new("GET", url.clone()))?;
            if response.status() == 404 {
                return Ok(Vec::new());
            }
            require_status(&response, &[200], &url)?;
            next = response
                .header("link")
                .and_then(next_link)
                .map(|path| self.absolute(&path));
            let page: TagList = serde_json::from_slice(&read_body(response, &url)?)
                .with_context(|| format!("{url} answered an unreadable tag list"))?;
            tags.extend(page.tags.unwrap_or_default());
        }
        Ok(tags)
    }

    pub fn manifest(&self, reference: &str) -> Result<Option<Fetched>> {
        let url = self.url(&format!("manifests/{reference}"));
        let mut request = Request::new("GET", url.clone());
        request.accept = Some(MANIFEST_MEDIA_TYPE);
        let response = self.call(request)?;
        if matches!(response.status(), 403 | 404) {
            return Ok(None);
        }
        require_status(&response, &[200], &url)?;
        let announced = response.header("docker-content-digest").map(str::to_string);
        let bytes = read_body(response, &url)?;
        let digest = sha256_digest(&bytes);
        if announced.is_some_and(|announced| announced != digest) {
            anyhow::bail!("{url} sent a manifest that does not match its digest");
        }
        Ok(Some(Fetched { digest, bytes }))
    }

    pub fn blob(&self, digest: &str, size: u64) -> Result<Vec<u8>> {
        let url = self.url(&format!("blobs/{digest}"));
        let response = self.call(Request::new("GET", url.clone()))?;
        require_status(&response, &[200], &url)?;
        let bytes = read_body(response, &url)?;
        if bytes.len() as u64 != size || sha256_digest(&bytes) != digest {
            anyhow::bail!("{url} sent a blob that does not match its digest and size");
        }
        Ok(bytes)
    }

    pub fn has_blob(&self, digest: &str) -> Result<bool> {
        let url = self.url(&format!("blobs/{digest}"));
        let response = self.call(Request::new("HEAD", url.clone()))?;
        require_status(&response, &[200, 404], &url)?;
        Ok(response.status() == 200)
    }

    pub fn upload_blob(&self, digest: &str, bytes: &[u8]) -> Result<()> {
        let start = self.url("blobs/uploads/");
        let response = self.call(Request::new("POST", start.clone()))?;
        require_status(&response, &[202], &start)?;
        let session = response
            .header("location")
            .with_context(|| format!("{start} gave no upload location"))?;
        let separator = if session.contains('?') { '&' } else { '?' };
        let url = format!("{}{separator}digest={digest}", self.absolute(session));
        let mut request = Request::new("PUT", url.clone());
        request.content_type = Some("application/octet-stream");
        request.body = Some(bytes);
        let response = self.call(request)?;
        require_status(&response, &[201], &url)
    }

    pub fn put_manifest(&self, tag: &str, bytes: &[u8]) -> Result<()> {
        let url = self.url(&format!("manifests/{tag}"));
        let mut request = Request::new("PUT", url.clone());
        request.content_type = Some(MANIFEST_MEDIA_TYPE);
        request.body = Some(bytes);
        let response = self.call(request)?;
        require_status(&response, &[201], &url)
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/v2/{}/{path}",
            self.location.url, self.location.repository
        )
    }

    fn absolute(&self, location: &str) -> String {
        if location.starts_with("http://") || location.starts_with("https://") {
            location.to_string()
        } else {
            format!("{}{location}", self.location.url)
        }
    }

    fn call(&self, request: Request) -> Result<ureq::Response> {
        let used = self.current_token();
        let response = self.send_once(&request, used.as_deref())?;
        if response.status() != 401 {
            return Ok(response);
        }
        let challenge = response
            .header("www-authenticate")
            .with_context(|| format!("{} asked for credentials without a challenge", request.url))?
            .to_string();
        let token = self.refresh_token(used, &challenge)?;
        self.send_once(&request, Some(&token))
    }

    fn send_once(&self, request: &Request, token: Option<&str>) -> Result<ureq::Response> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let mut call = self.agent.request(request.method, &request.url);
        if let Some(token) = token {
            call = call.set("Authorization", &format!("Bearer {token}"));
        }
        if let Some(accept) = request.accept {
            call = call.set("Accept", accept);
        }
        if let Some(content_type) = request.content_type {
            call = call.set("Content-Type", content_type);
        }
        send(call, request.body, &request.url)
    }

    fn current_token(&self) -> Option<String> {
        self.token.lock().ok().and_then(|token| token.clone())
    }

    fn refresh_token(&self, used: Option<String>, challenge: &str) -> Result<String> {
        let mut token = self
            .token
            .lock()
            .map_err(|_| anyhow::anyhow!("the registry token lock is poisoned"))?;
        if let Some(current) = token.as_ref() {
            if used.as_ref() != Some(current) {
                return Ok(current.clone());
            }
        }
        let fresh = self.fetch_token(challenge)?;
        *token = Some(fresh.clone());
        Ok(fresh)
    }

    fn fetch_token(&self, challenge: &str) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct TokenResponse {
            token: Option<String>,
            access_token: Option<String>,
        }
        let mut challenge = BearerChallenge::parse(challenge)?;
        let actions = match self.access {
            Access::Pull => "pull",
            Access::PullPush => "pull,push",
        };
        challenge.scope = Some(format!("repository:{}:{actions}", self.location.repository));
        let url = challenge.token_url(&self.location)?.to_string();
        let mut call = self.agent.get(&url);
        if let Some(credentials) = &self.credentials {
            let basic = base64::engine::general_purpose::STANDARD
                .encode(format!("{}:{}", credentials.username, credentials.password));
            call = call.set("Authorization", &format!("Basic {basic}"));
        }
        self.requests.fetch_add(1, Ordering::Relaxed);
        let response = send(call, None, &url)?;
        require_status(&response, &[200], &url)?;
        let body: TokenResponse = serde_json::from_slice(&read_body(response, &url)?)
            .context("the registry token response is unreadable")?;
        body.token
            .or(body.access_token)
            .context("the registry token response has no token")
    }
}

fn location(reference: &str) -> Result<RegistryLocation> {
    let (scheme, rest) = reference.split_once("://").unwrap_or(("https", reference));
    if scheme != "https" && scheme != "http" {
        anyhow::bail!("registry {reference} must use http or https");
    }
    let (host, repository) = rest
        .split_once('/')
        .filter(|(host, repository)| !host.is_empty() && !repository.is_empty())
        .with_context(|| format!("registry {reference} must name a host and a repository"))?;
    Ok(RegistryLocation {
        url: format!("{scheme}://{host}"),
        repository: repository.trim_end_matches('/').to_string(),
    })
}

fn require_status(response: &ureq::Response, expected: &[u16], url: &str) -> Result<()> {
    let status = response.status();
    if expected.contains(&status) {
        return Ok(());
    }
    anyhow::bail!("{url} answered {status} {}", response.status_text())
}

fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|link| {
        let (target, params) = link.split_once(';')?;
        params.contains("rel=\"next\"").then(|| {
            target
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_defaults_to_https_and_keeps_the_repository_path() {
        let cases: &[(&str, &str, &str)] = &[
            (
                "ghcr.io/qol-tools/plugins",
                "https://ghcr.io",
                "qol-tools/plugins",
            ),
            (
                "http://127.0.0.1:5000/qol-tools/plugins",
                "http://127.0.0.1:5000",
                "qol-tools/plugins",
            ),
        ];
        for (reference, url, repository) in cases {
            let location = location(reference).unwrap();
            assert_eq!(location.url, *url);
            assert_eq!(location.repository, *repository);
        }
        for bad in ["ghcr.io", "ftp://ghcr.io/a/b", "ghcr.io/"] {
            assert!(location(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn next_link_reads_only_the_next_relation() {
        let cases: &[(&str, Option<&str>)] = &[
            (
                r#"</v2/a/b/tags/list?n=2&last=x>; rel="next""#,
                Some("/v2/a/b/tags/list?n=2&last=x"),
            ),
            (r#"</v2/a/b/tags/list?n=2>; rel="prev""#, None),
        ];
        for (header, expected) in cases {
            assert_eq!(next_link(header).as_deref(), *expected, "{header}");
        }
    }
}
