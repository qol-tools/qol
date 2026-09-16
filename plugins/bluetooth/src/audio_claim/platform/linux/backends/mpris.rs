use std::collections::HashMap;

use dbus::blocking::stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged;
use dbus::message::SignalArgs;

use super::super::WATCH_RETRY_DELAY;

pub(in crate::audio_claim::platform::linux) fn spawn_media_play_watch(
    sender: tokio::sync::mpsc::UnboundedSender<()>,
) {
    tokio::spawn(async move {
        loop {
            let (resource, connection) = match dbus_tokio::connection::new_session_sync() {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("Bluetooth audio claim media watch cannot reach D-Bus: {error}");
                    qol_runtime::probe!(
                        "BLUETOOTH_AUDIO_CLAIM",
                        "event=media_watch outcome=exited reason=connect_failed"
                    );
                    tokio::time::sleep(WATCH_RETRY_DELAY).await;
                    continue;
                }
            };
            let resource = tokio::spawn(resource);
            let rule = PropertiesPropertiesChanged::match_rule(
                None,
                Some(&"/org/mpris/MediaPlayer2".into()),
            )
            .static_clone();
            let media_match = match connection.add_match(rule).await {
                Ok(media_match) => media_match,
                Err(error) => {
                    eprintln!(
                        "Bluetooth audio claim media watch cannot match MPRIS signals: {error}"
                    );
                    qol_runtime::probe!(
                        "BLUETOOTH_AUDIO_CLAIM",
                        "event=media_watch outcome=exited reason=match_failed"
                    );
                    resource.abort();
                    tokio::time::sleep(WATCH_RETRY_DELAY).await;
                    continue;
                }
            };
            let sender = sender.clone();
            let mut statuses = HashMap::<String, String>::new();
            let _media_match =
                media_match.cb(move |message, signal: PropertiesPropertiesChanged| {
                    if signal.interface_name != "org.mpris.MediaPlayer2.Player" {
                        return true;
                    }
                    let Some(status) = dbus::arg::prop_cast::<String>(
                        &signal.changed_properties,
                        "PlaybackStatus",
                    ) else {
                        return true;
                    };
                    let player = message
                        .sender()
                        .map(|name| name.to_string())
                        .unwrap_or_default();
                    if is_play_transition(&mut statuses, player, status) {
                        let _ = sender.send(());
                    }
                    true
                });
            qol_runtime::probe!("BLUETOOTH_AUDIO_CLAIM", "event=media_watch outcome=started");
            if let Ok(error) = resource.await {
                eprintln!("Bluetooth audio claim media watch lost D-Bus: {error}");
            }
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=media_watch outcome=exited reason=disconnected"
            );
            tokio::time::sleep(WATCH_RETRY_DELAY).await;
        }
    });
}

fn is_play_transition(
    statuses: &mut HashMap<String, String>,
    player: String,
    status: &str,
) -> bool {
    let previous = statuses.insert(player, status.to_string());
    status == "Playing" && previous.as_deref() != Some("Playing")
}

#[cfg(test)]
mod tests {
    use super::is_play_transition;
    use std::collections::HashMap;

    #[test]
    fn only_a_change_to_playing_counts_as_media_play() {
        let mut statuses = HashMap::new();
        assert!(is_play_transition(&mut statuses, ":1.1".into(), "Playing"));
        assert!(!is_play_transition(&mut statuses, ":1.1".into(), "Playing"));
        assert!(!is_play_transition(&mut statuses, ":1.1".into(), "Paused"));
        assert!(is_play_transition(&mut statuses, ":1.1".into(), "Playing"));
        assert!(is_play_transition(&mut statuses, ":1.2".into(), "Playing"));
    }
}
