use axum::Json;
use qol_apps::InstalledApp;
use serde_json::Value;

mod platform;
use platform::discover_installed_apps;

pub(in super::super) async fn list_apps() -> Json<Vec<Value>> {
    let apps = tokio::task::spawn_blocking(discover_installed_apps)
        .await
        .unwrap_or_default();
    Json(media_app_values(apps))
}

fn media_app_values(apps: Vec<InstalledApp>) -> Vec<Value> {
    let mut values = apps
        .into_iter()
        .filter_map(|app| {
            let bundle_id = app.bundle_id?.trim().to_string();
            if bundle_id.is_empty() {
                return None;
            }
            let name = app
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
                .unwrap_or(app.name);
            Some(serde_json::json!({ "bundle_id": bundle_id, "name": name }))
        })
        .collect::<Vec<_>>();
    values.sort_by_key(app_name);
    values.dedup_by(|left, right| left["bundle_id"] == right["bundle_id"]);
    values
}

fn app_name(value: &Value) -> String {
    value["name"].as_str().unwrap_or("").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn media_values_use_bundle_filename_and_dedupe_bundle_ids() {
        let apps = vec![
            InstalledApp {
                name: "Metadata Name".to_string(),
                bundle_id: Some("com.acme.shared".to_string()),
                path: PathBuf::from("/Applications/Zed Name.app"),
            },
            InstalledApp {
                name: "Other Metadata".to_string(),
                bundle_id: Some("com.acme.shared".to_string()),
                path: PathBuf::from("/Applications/Alpha Name.app"),
            },
        ];

        let values = media_app_values(apps);

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["name"], "Alpha Name");
        assert_eq!(values[0]["bundle_id"], "com.acme.shared");
    }

    #[test]
    fn media_values_drop_missing_and_blank_bundle_ids() {
        let apps = [None, Some("  ".to_string())]
            .into_iter()
            .enumerate()
            .map(|(index, bundle_id)| InstalledApp {
                name: format!("App {index}"),
                bundle_id,
                path: PathBuf::from(format!("/Applications/App {index}.app")),
            })
            .collect();

        assert!(media_app_values(apps).is_empty());
    }
}
