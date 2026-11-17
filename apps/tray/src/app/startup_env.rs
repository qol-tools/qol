pub(crate) fn drop_launching_claude_session_env() {
    for name in qol_agent_homes::inherited_claude_session_env() {
        std::env::remove_var(name);
    }
}

pub(crate) fn scrub_inherited_daemon_handoff_env() {
    for key in qol_conventions::daemon_handoff_env_keys() {
        std::env::remove_var(key);
    }
}
