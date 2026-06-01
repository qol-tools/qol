use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::storage::{delete_github_credential, github_auth_status, store_github_credential};
use super::types::{
    GitHubAuthSessionResponse, GitHubAuthSessionState, GitHubAuthStartResponse, GitHubAuthStatus,
    GitHubCredentialRecord, GitHubCredentialSource,
};

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const USER_URL: &str = "https://api.github.com/user";
const SESSION_TTL: Duration = Duration::from_secs(900);
const DEFAULT_OAUTH_CLIENT_ID: &str = "Ov23liCGJveN37QzFWGY";

pub(crate) struct GitHubAuthService {
    client: reqwest::Client,
    client_id: String,
    sessions: Mutex<HashMap<String, DeviceAuthSession>>,
}

#[derive(Debug, Clone)]
struct DeviceAuthSession {
    device_code: String,
    created_at: Instant,
    status: AuthSessionStatus,
}

#[derive(Debug, Clone)]
enum AuthSessionStatus {
    Pending,
    Authorized { login: String },
    Failed { message: String },
}

impl GitHubAuthService {
    pub(crate) fn new() -> Self {
        let client_id = std::env::var("QOL_TRAY_GITHUB_OAUTH_CLIENT_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_OAUTH_CLIENT_ID.to_string());
        Self {
            client: reqwest::Client::new(),
            client_id,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn status(&self) -> GitHubAuthStatus {
        github_auth_status()
    }

    pub(crate) async fn start(&self, scopes: &[String]) -> Result<GitHubAuthStartResponse> {
        #[derive(Deserialize)]
        struct DeviceCodeResponse {
            device_code: String,
            user_code: String,
            verification_uri: String,
            interval: Option<u64>,
        }

        if scopes.is_empty() {
            anyhow::bail!("no scopes provided for GitHub authorization");
        }
        let scope_param = scopes.join(",");

        let response = self
            .client
            .post(DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": self.client_id,
                "scope": scope_param,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub device code request failed: {status} {body}");
        }

        let body: DeviceCodeResponse = response.json().await?;
        let session_id = session_id();
        let interval = body.interval.unwrap_or(5);

        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        retain_live_sessions(&mut sessions);
        sessions.insert(
            session_id.clone(),
            DeviceAuthSession {
                device_code: body.device_code,
                created_at: Instant::now(),
                status: AuthSessionStatus::Pending,
            },
        );

        Ok(GitHubAuthStartResponse {
            session_id,
            user_code: body.user_code,
            verification_uri: body.verification_uri,
            interval,
        })
    }

    pub(crate) async fn poll_session(&self, session_id: &str) -> GitHubAuthSessionResponse {
        let device_code = {
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(session) = sessions.get(session_id) else {
                return GitHubAuthSessionResponse {
                    state: GitHubAuthSessionState::Failed,
                    login: None,
                    error: Some("GitHub auth session was not found".to_string()),
                };
            };
            if !matches!(session.status, AuthSessionStatus::Pending) {
                return session_response(session);
            }
            session.device_code.clone()
        };

        match self.exchange_device_code(&device_code).await {
            DeviceFlowResult::Pending => GitHubAuthSessionResponse {
                state: GitHubAuthSessionState::Pending,
                login: None,
                error: None,
            },
            DeviceFlowResult::Authorized(token) => {
                match self.finish_authorization(session_id, token).await {
                    Ok(login) => GitHubAuthSessionResponse {
                        state: GitHubAuthSessionState::Authorized,
                        login: Some(login),
                        error: None,
                    },
                    Err(error) => {
                        let message = error.to_string();
                        self.mark_failed(session_id, &message);
                        GitHubAuthSessionResponse {
                            state: GitHubAuthSessionState::Failed,
                            login: None,
                            error: Some(message),
                        }
                    }
                }
            }
            DeviceFlowResult::Failed(message) => {
                self.mark_failed(session_id, &message);
                GitHubAuthSessionResponse {
                    state: GitHubAuthSessionState::Failed,
                    login: None,
                    error: Some(message),
                }
            }
        }
    }

    pub(crate) fn disconnect(&self) -> Result<()> {
        delete_github_credential()
    }

    async fn exchange_device_code(&self, device_code: &str) -> DeviceFlowResult {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: Option<String>,
            scope: Option<String>,
            error: Option<String>,
        }

        let response = match self
            .client
            .post(ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": self.client_id,
                "device_code": device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return DeviceFlowResult::Failed(error.to_string()),
        };

        let body: TokenResponse = match response.json().await {
            Ok(body) => body,
            Err(error) => return DeviceFlowResult::Failed(error.to_string()),
        };

        if let Some(error) = &body.error {
            return match error.as_str() {
                "authorization_pending" | "slow_down" => DeviceFlowResult::Pending,
                "expired_token" => {
                    DeviceFlowResult::Failed("GitHub authorization expired".to_string())
                }
                "access_denied" => {
                    DeviceFlowResult::Failed("GitHub authorization was denied".to_string())
                }
                _ => DeviceFlowResult::Failed(format!("GitHub authorization failed: {error}")),
            };
        }

        match body.access_token.filter(|t| !t.trim().is_empty()) {
            Some(access_token) => DeviceFlowResult::Authorized(OAuthAccessToken {
                access_token,
                scopes: parse_scopes(body.scope.as_deref()),
            }),
            None => DeviceFlowResult::Failed("GitHub did not return an access token".to_string()),
        }
    }

    async fn finish_authorization(
        &self,
        session_id: &str,
        token: OAuthAccessToken,
    ) -> Result<String> {
        let login = self.fetch_login(&token.access_token).await?;
        store_github_credential(&GitHubCredentialRecord {
            access_token: token.access_token,
            source: GitHubCredentialSource::Oauth,
            login: Some(login.clone()),
            scopes: token.scopes,
        })?;
        run_post_auth_migrations().await;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(session) = sessions.get_mut(session_id) {
            session.status = AuthSessionStatus::Authorized {
                login: login.clone(),
            };
        }
        Ok(login)
    }

    async fn fetch_login(&self, access_token: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct UserResponse {
            login: String,
        }

        let response = self
            .client
            .get(USER_URL)
            .header("User-Agent", "qol-tray")
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub user lookup failed: {status} {body}");
        }
        let body: UserResponse = response.json().await?;
        if body.login.trim().is_empty() {
            anyhow::bail!("GitHub user lookup returned an empty login");
        }
        Ok(body.login)
    }

    fn mark_failed(&self, session_id: &str, message: &str) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(session) = sessions.get_mut(session_id) {
            session.status = AuthSessionStatus::Failed {
                message: message.to_string(),
            };
        }
    }
}

enum DeviceFlowResult {
    Pending,
    Authorized(OAuthAccessToken),
    Failed(String),
}

#[derive(Debug, Clone)]
struct OAuthAccessToken {
    access_token: String,
    scopes: Vec<String>,
}

async fn run_post_auth_migrations() {
    let config_dir = match crate::paths::shared_config_dir() {
        Ok(dir) => dir,
        Err(error) => {
            log::error!("post-auth migrations skipped: cannot resolve config dir: {error:#}");
            return;
        }
    };
    if let Err(error) = crate::migrations_startup::run_post_auth_if_authed(&config_dir).await {
        log::error!("post-auth migrations failed after credential store: {error:#}");
    }
}

fn parse_scopes(scope: Option<&str>) -> Vec<String> {
    let Some(scope) = scope else {
        return Vec::new();
    };
    scope
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn session_response(session: &DeviceAuthSession) -> GitHubAuthSessionResponse {
    match &session.status {
        AuthSessionStatus::Pending => GitHubAuthSessionResponse {
            state: GitHubAuthSessionState::Pending,
            login: None,
            error: None,
        },
        AuthSessionStatus::Authorized { login } => GitHubAuthSessionResponse {
            state: GitHubAuthSessionState::Authorized,
            login: Some(login.clone()),
            error: None,
        },
        AuthSessionStatus::Failed { message } => GitHubAuthSessionResponse {
            state: GitHubAuthSessionState::Failed,
            login: None,
            error: Some(message.clone()),
        },
    }
}

fn retain_live_sessions(sessions: &mut HashMap<String, DeviceAuthSession>) {
    sessions.retain(|_, session| session.created_at.elapsed() < SESSION_TTL);
}

fn session_id() -> String {
    use base64::Engine;
    use rand::TryRngCore;

    let mut raw = [0u8; 24];
    rand::rngs::OsRng
        .try_fill_bytes(&mut raw)
        .expect("OS random number generator failed");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parse_scopes_cases() {
        let cases: Vec<(Option<&str>, Vec<&str>)> = vec![
            (None, vec![]),
            (Some(""), vec![]),
            (Some("repo"), vec!["repo"]),
            (Some("repo,gist"), vec!["repo", "gist"]),
            (Some("repo , gist"), vec!["repo", "gist"]),
            (Some("repo,,gist"), vec!["repo", "gist"]),
            (Some("repo,"), vec!["repo"]),
            (Some(",repo"), vec!["repo"]),
            (Some("   ,  , "), vec![]),
            (Some(",,,"), vec![]),
            (Some("repo,gist,read:org"), vec!["repo", "gist", "read:org"]),
        ];
        for (input, expected) in cases {
            let result = parse_scopes(input);
            let expected: Vec<String> = expected.into_iter().map(String::from).collect();
            assert_eq!(result, expected, "input: {input:?}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_parse_scopes_never_returns_empty_strings(input in ".*") {
            let result = parse_scopes(Some(&input));
            for scope in &result {
                assert!(!scope.is_empty());
                assert_eq!(scope.trim(), scope.as_str());
            }
        }

        #[test]
        fn prop_parse_scopes_none_returns_empty(_dummy in 0..1u8) {
            assert!(parse_scopes(None).is_empty());
        }

        #[test]
        fn prop_parse_scopes_count_bounded(input in "[a-z,]{0,50}") {
            let result = parse_scopes(Some(&input));
            let comma_count = input.chars().filter(|&ch| ch == ',').count();
            assert!(result.len() <= comma_count + 1);
        }
    }
}
