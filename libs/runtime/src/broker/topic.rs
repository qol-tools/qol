//! Topic-access capability gate for the inter-plugin event bus.
//!
//! `check_topic_access` runs the access decision against the plugin's
//! declared topic set, mirroring `field::check_field_access` for the
//! pane-field pull API. Unlike the closed `PaneField` vocabulary,
//! topics are open-set strings: the declared set comes from the plugin
//! manifest (the `pane-fields` declaration pattern) and the broker
//! refuses any topic that is not declared, for both publish and
//! subscribe.
//!
//! See the broker ADR
//! (docs/adr/RUNTIME-1-af-unix-broker-with-peer-cred-auth-and-pull-pane-f.md).

/// Reason a topic-access check refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicCheckError {
    /// The requested topic is not in the plugin's declared topic set.
    NotDeclared { topic: String },
}

impl std::fmt::Display for TopicCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopicCheckError::NotDeclared { topic } => {
                write!(f, "topic {topic:?} not declared in the plugin's topic set")
            }
        }
    }
}

impl std::error::Error for TopicCheckError {}

/// Decide whether `requested` may be published or subscribed given the
/// plugin's `declared` topic set.
///
/// Unlike the pane-field gate there is no always-allowed set: nothing
/// on the bus is implicitly public, so a plugin with no declared
/// topics can neither publish nor subscribe.
pub fn check_topic_access(declared: &[String], requested: &str) -> Result<(), TopicCheckError> {
    if declared.iter().any(|topic| topic == requested) {
        return Ok(());
    }
    Err(TopicCheckError::NotDeclared {
        topic: requested.to_string(),
    })
}
