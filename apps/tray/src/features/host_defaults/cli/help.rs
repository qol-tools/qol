use super::parse::Topic;

const GENERAL: &str = "\
qol-tray defaults - inspect and manage the host defaults a plugin can take over

USAGE:
    qol-tray defaults list [--json]
    qol-tray defaults probe <role> [--json]
    qol-tray defaults claim <role> --plugin <id> [--json]
    qol-tray defaults release <role> [--json]
    qol-tray defaults open <role> <path-or-uri>... [--json]
    qol-tray defaults help [<command>]

ROLES:
    folder.open, folder.reveal, slot.launcher, slot.window_switcher,
    slot.screenshot, slot.clipboard_history, slot.dictation

OUTPUT:
    Human text goes to stdout. --json prints one JSON document to stdout and
    nothing else. Diagnostics go to stderr.

EXIT:
    0 on success, 1 when the host refused or an operation failed, 2 for a
    malformed command line.";

const LIST: &str = "\
qol-tray defaults list [--json]

Lists every role an installed plugin declares in plugin.toml, with what the
host allows (silent, needs confirmation, impossible, unknown), who holds the
role in the ledger, and which lifetimes the host can honour.

JSON: {\"roles\": [{\"role\", \"plugins\", \"holder\", \"state\", \"owner\",
\"triggers\", \"kept\", \"lifetimes\"}]}
EXIT: 0, or 1 when the ledger cannot be read.";

const PROBE: &str = "\
qol-tray defaults probe <role> [--json]

Asks the host what it would allow for one role and who owns the job today.
Nothing is changed. An Unknown state means the probe itself failed.

JSON: {\"role\", \"state\": {\"kind\", \"detail\"}, \"owner\": {\"kind\", \"name\"},
\"triggers\", \"kept\", \"lifetimes\"}
EXIT: 0, 2 for an unknown role.";

const CLAIM: &str = "\
qol-tray defaults claim <role> --plugin <id> [--json]

Hands the role to an installed plugin that declares it in [[default_for]].
The claim is refused, naming the reason, unless the host allows it silently
for the current lifetime. Slot roles are always a portable session: they bind
the plugin's action to the role's keys and free the system binding, and the
system binding comes back when qol stops, or on the next start after a
crash. Handler roles register qol's opener and follow the host's residency
(portable session, or resident policy on a resident host).

JSON: {\"role\", \"plugin\", \"claimed\": true}
EXIT: 0 claimed, 1 refused or failed, 2 for a malformed command line.";

const RELEASE: &str = "\
qol-tray defaults release <role> [--json]

Gives the role back to the host. Only what qol wrote is undone: a value or
binding you changed since is left alone. For a slot role it also turns off,
on this OS only, the plugin's own enabled bindings on every key the system
tool answers (Alt+Tab and Alt+Shift+Tab for the window switcher), so qol does
not free the system key again on the next start. Claiming the role again turns
the extra keys' bindings back on.

JSON: {\"role\", \"outcome\": \"restored\" | \"left_user_change\" | \"already_gone\"}
EXIT: 0, 1 when restoring failed.";

const OPEN: &str = "\
qol-tray defaults open <role> <path-or-uri>... [--json]

What the registered opener runs. Finds the plugin holding the role in the
ledger and starts its declared action with the paths appended; file:// uris
are converted to paths. With no holder it fails instead of handing the
request back to the system handler.

JSON: {\"role\", \"plugin\", \"args\"}
EXIT: 0 started, 1 when nobody holds the role or the plugin cannot start.";

pub(super) fn text(topic: Topic) -> &'static str {
    match topic {
        Topic::General => GENERAL,
        Topic::List => LIST,
        Topic::Probe => PROBE,
        Topic::Claim => CLAIM,
        Topic::Release => RELEASE,
        Topic::Open => OPEN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_documents_output_and_exit() {
        for topic in [
            Topic::List,
            Topic::Probe,
            Topic::Claim,
            Topic::Release,
            Topic::Open,
        ] {
            let help = text(topic);
            assert!(help.contains("EXIT"), "{topic:?}");
            assert!(help.contains("JSON"), "{topic:?}");
        }
        assert!(text(Topic::General).contains("OUTPUT"));
        assert!(text(Topic::General).contains("EXIT"));
    }
}
