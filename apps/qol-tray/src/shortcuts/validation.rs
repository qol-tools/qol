use super::model::{AppRef, Shortcut, ShortcutAction};

const MAX_NAME_LEN: usize = 128;
const MAX_URL_LEN: usize = 2048;
const MAX_PATH_LEN: usize = 1024;

pub fn validate_shortcut(shortcut: &Shortcut) -> Result<(), String> {
    validate_id(&shortcut.id)?;
    validate_name(&shortcut.name)?;
    validate_action(&shortcut.action)
}

pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id must not be empty".into());
    }
    if id.len() > 64 {
        return Err("id must be at most 64 characters".into());
    }
    if id.starts_with('-') {
        return Err("id must not start with '-'".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("id must only contain [A-Za-z0-9_-]".into());
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("name must not be empty".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("name must be at most {} characters", MAX_NAME_LEN));
    }
    reject_null_bytes(name, "name")
}

fn validate_action(action: &ShortcutAction) -> Result<(), String> {
    match action {
        ShortcutAction::OpenUrl {
            url,
            browser_override,
        } => {
            validate_url(url)?;
            if let Some(app_ref) = browser_override {
                validate_app_ref(app_ref, "browser_override")?;
            }
            Ok(())
        }
        ShortcutAction::LaunchApp { app } => validate_app_ref(app, "app"),
    }
}

fn validate_url(url: &str) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("url must not be empty".into());
    }
    if url.len() > MAX_URL_LEN {
        return Err(format!("url must be at most {} characters", MAX_URL_LEN));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("url must start with http:// or https://".into());
    }
    reject_null_bytes(url, "url")
}

fn validate_app_ref(app_ref: &AppRef, field: &str) -> Result<(), String> {
    match app_ref {
        AppRef::BundleId { id } => {
            if id.trim().is_empty() {
                return Err(format!("{} bundle_id must not be empty", field));
            }
            reject_null_bytes(id, field)
        }
        AppRef::Path { path } => {
            if path.trim().is_empty() {
                return Err(format!("{} path must not be empty", field));
            }
            if path.len() > MAX_PATH_LEN {
                return Err(format!(
                    "{} path must be at most {} characters",
                    field, MAX_PATH_LEN
                ));
            }
            reject_null_bytes(path, field)?;
            reject_traversal(path, field)
        }
        AppRef::Name { name } => {
            if name.trim().is_empty() {
                return Err(format!("{} name must not be empty", field));
            }
            reject_null_bytes(name, field)
        }
    }
}

fn reject_null_bytes(s: &str, field: &str) -> Result<(), String> {
    if s.contains('\0') {
        return Err(format!("{} must not contain null bytes", field));
    }
    Ok(())
}

fn reject_traversal(path: &str, field: &str) -> Result<(), String> {
    if path.contains("..") {
        return Err(format!("{} must not contain path traversal", field));
    }
    Ok(())
}
