use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

static NAMES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub(super) fn display_name(bundle_id: &str) -> String {
    let mut names = NAMES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    names
        .get_or_insert_with(HashMap::new)
        .entry(bundle_id.to_string())
        .or_insert_with(|| installed_name(bundle_id).unwrap_or_else(|| fallback_name(bundle_id)))
        .clone()
}

fn installed_name(bundle_id: &str) -> Option<String> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    let path = objc2::rc::autoreleasepool(|_pool| {
        let url = NSWorkspace::sharedWorkspace()
            .URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?;
        Some(url.path()?.to_string())
    })?;
    qol_apps::read_macos_app_bundle(PathBuf::from(path)).map(|app| app.name)
}

fn fallback_name(bundle_id: &str) -> String {
    bundle_id
        .rsplit('.')
        .find(|part| !part.is_empty())
        .unwrap_or(bundle_id)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_name_cases() {
        let cases = [
            ("com.microsoft.teams2", "teams2"),
            ("Slack", "Slack"),
            ("com.example.", "example"),
            ("", ""),
        ];
        for (input, want) in cases {
            assert_eq!(fallback_name(input), want, "input {input:?}");
        }
    }
}
