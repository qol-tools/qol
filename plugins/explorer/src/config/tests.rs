use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;

#[test]
fn contract_defaults_match_runtime_type() {
    qol_config::validate_contract_defaults_match_type::<ExplorerConfig>(CONFIG_CONTRACT)
        .expect("contract defaults must match ExplorerConfig");
    let defaults: ExplorerConfig = qol_config::typed_defaults_from_contract(CONFIG_CONTRACT)
        .expect("contract defaults must parse");
    assert_eq!(defaults, ExplorerConfig::default());
}

#[test]
fn every_view_size_round_trips_through_its_contract_option() {
    let cases = [
        (ViewSize::Compact, "compact"),
        (ViewSize::List, "list"),
        (ViewSize::Roomy, "roomy"),
        (ViewSize::Icons, "icons"),
        (ViewSize::BigIcons, "big_icons"),
    ];
    let spec = qol_config::contract::parse_spec_str(CONFIG_CONTRACT).expect("contract parses");
    let options = &spec.field("view_size").expect("view_size field").options;
    assert_eq!(
        options,
        &cases.map(|(_, option)| option.to_string()).to_vec()
    );
    for (size, option) in cases {
        assert_eq!(serde_json::to_value(size).unwrap(), json!(option));
        assert_eq!(
            serde_json::from_value::<ViewSize>(json!(option)).unwrap(),
            size
        );
    }
}

#[test]
fn a_saver_sends_every_key_and_the_last_value_of_each_wins() {
    let written = Arc::new(Mutex::new(Map::new()));
    let sink = written.clone();
    let saver = ConfigSaver::with_writer(
        move |keys| {
            sink.lock()
                .unwrap()
                .extend(keys.as_object().unwrap().clone());
        },
        ExplorerConfig::default,
    );
    saver.save("view_size", ViewSize::Icons);
    saver.save("show_hidden", true);
    saver.save("view_size", ViewSize::Compact);
    drop(saver);
    assert_eq!(
        Value::Object(written.lock().unwrap().clone()),
        json!({"view_size": "compact", "show_hidden": true})
    );
}

#[test]
fn a_reload_queued_after_saves_answers_after_their_write() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let written = log.clone();
    let read = log.clone();
    let saver = ConfigSaver::with_writer(
        move |keys| written.lock().unwrap().push(keys.clone()),
        move || {
            read.lock().unwrap().push(json!("read"));
            ExplorerConfig {
                show_hidden: true,
                ..ExplorerConfig::default()
            }
        },
    );
    saver.save("view_size", ViewSize::Icons);
    saver.save("show_hidden", true);
    let answer = saver.reload().expect("the saver is running");
    let config = futures::executor::block_on(answer).expect("the reload answers");
    assert!(config.show_hidden);
    assert_eq!(
        *log.lock().unwrap(),
        [
            json!({"view_size": "icons", "show_hidden": true}),
            json!("read")
        ]
    );
}

#[test]
fn saves_queued_after_a_reload_are_written_after_its_read() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let written = log.clone();
    let read = log.clone();
    let saver = ConfigSaver::with_writer(
        move |keys| written.lock().unwrap().push(keys.clone()),
        move || {
            read.lock().unwrap().push(json!("read"));
            ExplorerConfig::default()
        },
    );
    saver.save("show_hidden", true);
    let answer = saver.reload().expect("the saver is running");
    saver.save("view_size", ViewSize::Roomy);
    futures::executor::block_on(answer).expect("the reload answers");
    drop(saver);
    assert_eq!(
        *log.lock().unwrap(),
        [
            json!({"show_hidden": true}),
            json!("read"),
            json!({"view_size": "roomy"})
        ]
    );
}

#[test]
fn default_pins_list_home_then_the_usual_folders() {
    let pins: Vec<(String, String)> = default_pins()
        .into_iter()
        .map(|pin| (pin.name, pin.path))
        .collect();
    let expected: Vec<(String, String)> = [
        ("Home", "~"),
        ("Desktop", "~/Desktop"),
        ("Documents", "~/Documents"),
        ("Downloads", "~/Downloads"),
        ("Music", "~/Music"),
        ("Pictures", "~/Pictures"),
        ("Videos", "~/Videos"),
    ]
    .map(|(name, path)| (name.to_string(), path.to_string()))
    .to_vec();
    assert_eq!(pins, expected);
}
