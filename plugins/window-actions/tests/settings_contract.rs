use std::fs;
use std::path::PathBuf;

fn contract() -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("qol-config.toml");
    let body =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    toml::from_str(&body).unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e))
}

#[test]
fn single_field_groups_do_not_repeat_their_caption_on_the_row() {
    let contract = contract();
    let sections = contract["section"].as_table().expect("sections");
    let fields = contract["field"].as_table().expect("fields");
    for (id, section) in sections {
        let caption = section.get("description").and_then(|d| d.as_str());
        let rows: Vec<_> = fields
            .iter()
            .filter(|(_, f)| f.get("section").and_then(|s| s.as_str()) == Some(id))
            .collect();
        if rows.len() != 1 {
            continue;
        }
        let row = rows[0].1.get("description").and_then(|d| d.as_str());
        assert!(
            row.is_none() || row != caption,
            "section {id} repeats its caption on its only row"
        );
    }
}

#[test]
fn snap_and_glide_rows_carry_no_description() {
    let contract = contract();
    for id in ["snap_fraction", "glide_speed_px_per_second"] {
        assert!(
            contract["field"][id].get("description").is_none(),
            "{id} must rely on its group caption"
        );
    }
}
