use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use serde::Serialize;

use super::service::{has_github_access, SyncService};
use crate::features::auth::{cumulative_scopes_for, AuthProvider};
use crate::features::github_auth::{GitHubAuthService, GitHubAuthSessionState};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum GitHubConnectState {
    #[default]
    Idle,
    Waiting {
        user_code: String,
        verification_uri: String,
    },
    Connecting,
    Failed {
        message: String,
    },
}

pub(crate) struct GitHubConnect {
    auth: Arc<GitHubAuthService>,
    sync: Arc<SyncService>,
    state: Mutex<GitHubConnectState>,
}

impl GitHubConnect {
    pub(crate) fn new(auth: Arc<GitHubAuthService>, sync: Arc<SyncService>) -> Self {
        Self {
            auth,
            sync,
            state: Mutex::new(GitHubConnectState::Idle),
        }
    }

    pub(crate) fn state(&self) -> GitHubConnectState {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set(&self, next: GitHubConnectState) {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
    }

    pub(crate) async fn start(self: &Arc<Self>) -> Result<GitHubConnectState> {
        if matches!(
            self.state(),
            GitHubConnectState::Waiting { .. } | GitHubConnectState::Connecting
        ) {
            return Ok(self.state());
        }
        if has_github_access() {
            self.set(GitHubConnectState::Connecting);
            tokio::spawn(Arc::clone(self).bootstrap());
            return Ok(self.state());
        }
        let started = self
            .auth
            .start(&cumulative_scopes_for(AuthProvider::GitHub))
            .await?;
        self.set(GitHubConnectState::Waiting {
            user_code: started.user_code,
            verification_uri: started.verification_uri,
        });
        let interval = Duration::from_secs(started.interval.max(1));
        tokio::spawn(Arc::clone(self).watch(started.session_id, interval));
        Ok(self.state())
    }

    async fn watch(self: Arc<Self>, session_id: String, interval: Duration) {
        loop {
            tokio::time::sleep(interval).await;
            let poll = self.auth.poll_session(&session_id).await;
            match poll.state {
                GitHubAuthSessionState::Pending => {}
                GitHubAuthSessionState::Authorized => {
                    self.set(GitHubConnectState::Connecting);
                    return self.bootstrap().await;
                }
                GitHubAuthSessionState::Failed => {
                    return self.set(GitHubConnectState::Failed {
                        message: poll
                            .error
                            .unwrap_or_else(|| "GitHub did not accept the sign-in.".to_string()),
                    });
                }
            }
        }
    }

    async fn bootstrap(self: Arc<Self>) {
        let next = match self.sync.bootstrap_github_connect().await {
            Ok(_) => GitHubConnectState::Idle,
            Err(error) => GitHubConnectState::Failed {
                message: format!("{error:#}"),
            },
        };
        self.set(next);
    }
}
