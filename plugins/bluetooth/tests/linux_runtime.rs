#![cfg(target_os = "linux")]

#[path = "support/bluez.rs"]
mod bluez;

use std::time::Duration;

use bluez::{wait, Fixture};

const RESPONSIVE: Duration = Duration::from_secs(1);

#[test]
fn search_starts_while_automatic_connection_is_waiting_for_bluez() {
    let fixture = Fixture::start(true, false, false);
    assert!(wait(
        || fixture.state.lock().unwrap().stalled > 0,
        Duration::from_secs(3)
    ));
    fixture.action("start_search");
    assert!(
        wait(|| fixture.state.lock().unwrap().searching, RESPONSIVE),
        "Start search was blocked by an automatic connection"
    );
    assert_eq!(fixture.action("search_status")["searching"], true);
}

#[test]
fn stop_search_releases_discovery_while_disconnect_is_waiting_for_bluez() {
    let fixture = Fixture::start(false, false, true);
    fixture.action("start_search");
    assert!(wait(|| fixture.state.lock().unwrap().searching, RESPONSIVE));
    fixture.action("disconnect_device");
    assert!(wait(
        || fixture.state.lock().unwrap().stalled > 0,
        RESPONSIVE
    ));
    fixture.action("stop_search");
    assert!(
        wait(|| !fixture.state.lock().unwrap().searching, RESPONSIVE),
        "Stop search changed its label but left BlueZ scanning"
    );
    assert_eq!(fixture.action("search_status")["searching"], false);
}

#[test]
fn cached_audio_device_is_rediscovered_before_pairing() {
    let fixture = Fixture::start(false, true, false);
    fixture.action("start_search");
    assert!(wait(
        || fixture.action("search_status")["discovered_count"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        RESPONSIVE
    ));
    fixture.action("stop_search");
    assert!(wait(
        || !fixture.state.lock().unwrap().searching,
        RESPONSIVE
    ));
    fixture.remove();
    fixture.action("pair_device");
    assert!(
        wait(
            || fixture.state.lock().unwrap().pair_calls == 1,
            Duration::from_secs(3)
        ),
        "Pairing used the expired BlueZ object instead of rediscovering the cached device"
    );
    assert_eq!(fixture.state.lock().unwrap().starts, 2);
}

#[test]
fn pairing_bonds_through_a_non_pairable_adapter_and_restores_it() {
    let fixture = Fixture::start(false, true, false);
    fixture.action("start_search");
    assert!(wait(
        || fixture.action("search_status")["discovered_count"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        RESPONSIVE
    ));
    fixture.action("pair_device");
    assert!(wait(
        || fixture.state.lock().unwrap().pair_calls == 1,
        Duration::from_secs(3)
    ));
    let paired_while_pairable = fixture.state.lock().unwrap().paired_while_pairable;
    assert_eq!(
        paired_while_pairable,
        Some(true),
        "Pair ran while the adapter was not pairable, so the kernel paired without bonding"
    );
    assert!(
        wait(|| !fixture.state.lock().unwrap().pairable, RESPONSIVE),
        "the adapter was left pairable after the pairing finished"
    );
}

#[test]
fn search_status_reports_searching_as_soon_as_scan_is_acknowledged() {
    let fixture = Fixture::start(false, false, false);
    fixture.action("start_search");
    assert_eq!(
        fixture.action("search_status")["searching"],
        true,
        "the settings panel refreshes right after the acknowledgement and showed Scan again"
    );
}

fn search_stays_responsive_while(action: &str) {
    let fixture = Fixture::start(false, false, false);
    fixture.action(action);
    assert!(
        wait(
            || fixture.state.lock().unwrap().stalled > 0,
            Duration::from_secs(3)
        ),
        "{action} never reached BlueZ"
    );
    fixture.action("start_search");
    assert!(
        wait(|| fixture.state.lock().unwrap().searching, RESPONSIVE),
        "Start search was blocked by {action}"
    );
}

#[test]
fn search_starts_while_a_manual_reconnect_is_waiting_for_bluez() {
    search_stays_responsive_while("reconnect_trusted");
}

#[test]
fn search_starts_while_removal_is_waiting_for_bluez() {
    search_stays_responsive_while("remove_device");
}

#[test]
fn search_starts_while_untrust_is_waiting_for_bluez() {
    search_stays_responsive_while("untrust_device");
}
