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
    let saver = ConfigSaver::with_writer(move |keys| {
        sink.lock()
            .unwrap()
            .extend(keys.as_object().unwrap().clone());
    });
    saver.save("view_size", ViewSize::Icons);
    saver.save("show_hidden", true);
    saver.save("view_size", ViewSize::Compact);
    drop(saver);
    assert_eq!(
        Value::Object(written.lock().unwrap().clone()),
        json!({"view_size": "compact", "show_hidden": true})
    );
}
