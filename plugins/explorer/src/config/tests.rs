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
            Ok(())
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
        move |keys| {
            written.lock().unwrap().push(keys.clone());
            Ok(())
        },
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
    let log = log.lock().unwrap();
    let (answer, writes) = log.split_last().expect("the saver logged");
    assert_eq!(answer, &json!("read"));
    let merged = writes
        .iter()
        .flat_map(|write| write.as_object().expect("writes are objects").clone())
        .collect::<Map<String, Value>>();
    assert_eq!(
        Value::Object(merged),
        json!({"view_size": "icons", "show_hidden": true})
    );
}

#[test]
fn saves_queued_after_a_reload_are_written_after_its_read() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let written = log.clone();
    let read = log.clone();
    let saver = ConfigSaver::with_writer(
        move |keys| {
            written.lock().unwrap().push(keys.clone());
            Ok(())
        },
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
fn a_stored_pin_missing_its_name_or_path_still_loads_the_rest_of_the_config() {
    let config: ExplorerConfig = serde_json::from_value(json!({
        "view_size": "icons",
        "show_hidden": true,
        "pins": [{"name": "Only a name"}, {"path": "/srv/only-a-path"}, {}],
    }))
    .expect("a partial pin must not fail the config");
    assert_eq!(config.view_size, ViewSize::Icons);
    assert!(config.show_hidden);
    let pin = |name: &str, path: &str| Pin {
        name: name.to_string(),
        path: path.to_string(),
    };
    assert_eq!(
        config.pins,
        [
            pin("Only a name", ""),
            pin("", "/srv/only-a-path"),
            pin("", "")
        ]
    );
}

#[test]
fn a_failed_save_names_its_keys_once_and_taking_them_clears_the_list() {
    let saver = ConfigSaver::with_writer(
        |keys| match keys.get("pins") {
            Some(_) => Err("host refused".to_string()),
            None => Ok(()),
        },
        ExplorerConfig::default,
    );
    saver.save("pins", Vec::<Pin>::new());
    futures::executor::block_on(saver.reload().expect("the saver is running"))
        .expect("the reload answers");
    saver.save("view_size", ViewSize::Icons);
    saver.save("pins", Vec::<Pin>::new());
    futures::executor::block_on(saver.reload().expect("the saver is running"))
        .expect("the reload answers");

    assert_eq!(saver.take_failures(), ["pins"]);
    assert!(saver.take_failures().is_empty());
}
