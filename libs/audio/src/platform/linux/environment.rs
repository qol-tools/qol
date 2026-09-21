use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::AudioError;

pub(super) const COOKIE_LENGTH: usize = 256;
const SYSTEM_CLIENT_CONFIG: &str = "/etc/pulse/client.conf";
const CLIENT_CONFIG_NAME: &str = "client.conf";
const MAX_CONFIG_FILE_BYTES: u64 = 64 * 1024;
const MAX_CONFIG_FILES: usize = 64;
const MAX_INCLUDE_DEPTH: usize = 8;

#[derive(Debug)]
pub(super) struct SelectionInputs {
    pub(super) client_config: Option<PathBuf>,
    pub(super) config_path: Option<PathBuf>,
    pub(super) home: Option<PathBuf>,
    pub(super) xdg_config_home: Option<PathBuf>,
    pub(super) server: Option<String>,
    pub(super) runtime_path: Option<PathBuf>,
    pub(super) xdg_runtime: Option<PathBuf>,
    pub(super) cookie: Option<PathBuf>,
    pub(super) system_config: PathBuf,
}

#[derive(Debug, Default)]
pub(super) struct ClientConfig {
    pub(super) default_server: Option<String>,
    pub(super) cookie_file: Option<PathBuf>,
}

#[derive(Debug)]
pub(super) struct ClientSettings {
    pub(super) socket: PathBuf,
    pub(super) cookie: [u8; COOKIE_LENGTH],
}

pub(super) fn from_env() -> SelectionInputs {
    SelectionInputs {
        client_config: non_empty(std::env::var_os("PULSE_CLIENTCONFIG")).map(PathBuf::from),
        config_path: non_empty(std::env::var_os("PULSE_CONFIG_PATH")).map(PathBuf::from),
        home: non_empty(std::env::var_os("HOME")).map(PathBuf::from),
        xdg_config_home: non_empty(std::env::var_os("XDG_CONFIG_HOME")).map(PathBuf::from),
        server: non_empty(std::env::var_os("PULSE_SERVER"))
            .map(|value| value.to_string_lossy().into_owned()),
        runtime_path: non_empty(std::env::var_os("PULSE_RUNTIME_PATH")).map(PathBuf::from),
        xdg_runtime: non_empty(std::env::var_os("XDG_RUNTIME_DIR")).map(PathBuf::from),
        cookie: non_empty(std::env::var_os("PULSE_COOKIE")).map(PathBuf::from),
        system_config: PathBuf::from(SYSTEM_CLIENT_CONFIG),
    }
}

pub(super) fn load() -> Result<ClientSettings, AudioError> {
    resolve(&from_env())
}

pub(super) fn resolve(inputs: &SelectionInputs) -> Result<ClientSettings, AudioError> {
    let config = client_config(inputs)?;
    Ok(ClientSettings {
        socket: resolve_endpoint(inputs, &config)?,
        cookie: resolve_cookie_selection(inputs, &config)?,
    })
}

pub(super) fn client_config(inputs: &SelectionInputs) -> Result<ClientConfig, AudioError> {
    let mut config = ClientConfig::default();
    let mut files = 0usize;
    let mut stack = Vec::new();
    if let Some(path) = select_client_config(inputs)? {
        parse_client_config(&path, &mut config, &mut stack, &mut files, true)?;
    }
    Ok(config)
}

fn resolve_endpoint(
    inputs: &SelectionInputs,
    config: &ClientConfig,
) -> Result<PathBuf, AudioError> {
    if let Some(server) = &inputs.server {
        return resolve_socket(Some(server.clone()), None, None);
    }
    if let Some(server) = &config.default_server {
        return resolve_socket(Some(server.clone()), None, None);
    }
    resolve_socket(
        None,
        inputs.runtime_path.clone(),
        inputs.xdg_runtime.clone(),
    )
}

fn resolve_cookie_selection(
    inputs: &SelectionInputs,
    config: &ClientConfig,
) -> Result<[u8; COOKIE_LENGTH], AudioError> {
    if let Some(explicit) = &inputs.cookie {
        return resolve_cookie(
            Some(explicit.clone()),
            inputs.xdg_config_home.clone(),
            inputs.home.clone(),
        );
    }
    if let Some(configured) = &config.cookie_file {
        let path = if configured.is_absolute() {
            configured.clone()
        } else {
            let config_dir = config_dir(inputs.xdg_config_home.as_deref(), inputs.home.as_deref())
                .ok_or_else(|| {
                    AudioError::Authentication(
                        "a relative cookie-file needs a config home directory".to_owned(),
                    )
                })?;
            config_dir.join(configured)
        };
        return read_cookie(&path, "the configured cookie file");
    }
    resolve_cookie(None, inputs.xdg_config_home.clone(), inputs.home.clone())
}

fn select_client_config(inputs: &SelectionInputs) -> Result<Option<PathBuf>, AudioError> {
    if let Some(explicit) = &inputs.client_config {
        return match regular_file(explicit) {
            Ok(true) => Ok(Some(explicit.clone())),
            Ok(false) => Err(config_error(
                "PULSE_CLIENTCONFIG does not name a regular file",
            )),
            Err(error) => Err(config_error(format!(
                "PULSE_CLIENTCONFIG could not be read: {error}"
            ))),
        };
    }

    if let Some(config_path) = &inputs.config_path {
        let candidate = config_path.join(CLIENT_CONFIG_NAME);
        match regular_file(&candidate) {
            Ok(true) => return Ok(Some(candidate)),
            Ok(false) => {
                return Err(config_error(
                    "the PULSE_CONFIG_PATH client.conf is not a regular file",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(config_error(format!(
                    "the PULSE_CONFIG_PATH client.conf could not be read: {error}"
                )))
            }
        }
    } else if let Some(home) = &inputs.home {
        for relative in [".pulse/client.conf", ".config/pulse/client.conf"] {
            let candidate = home.join(relative);
            match regular_file(&candidate) {
                Ok(true) => return Ok(Some(candidate)),
                Ok(false) => {
                    return Err(config_error("the user client.conf is not a regular file"))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(config_error(format!(
                        "the user client.conf could not be read: {error}"
                    )))
                }
            }
        }
    }

    match regular_file(&inputs.system_config) {
        Ok(true) => Ok(Some(inputs.system_config.clone())),
        Ok(false) => Err(config_error("the system client.conf is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(config_error(format!(
            "the system client.conf could not be read: {error}"
        ))),
    }
}

fn parse_client_config(
    path: &Path,
    config: &mut ClientConfig,
    stack: &mut Vec<PathBuf>,
    files: &mut usize,
    use_drop_ins: bool,
) -> Result<(), AudioError> {
    if stack.len() >= MAX_INCLUDE_DEPTH {
        return Err(config_error(
            "the client configuration include depth is too deep",
        ));
    }
    *files += 1;
    if *files > MAX_CONFIG_FILES {
        return Err(config_error(
            "the client configuration spans too many files",
        ));
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        config_error(format!(
            "the client configuration could not be read: {error}"
        ))
    })?;
    if stack.contains(&canonical) {
        return Err(config_error("the client configuration includes itself"));
    }
    let text = read_config_text(path)?;
    stack.push(canonical);
    let parsed = parse_config_text(path, &text, config, stack, files);
    stack.pop();
    parsed?;

    if !use_drop_ins {
        return Ok(());
    }
    let Some(file_name) = path.file_name() else {
        return Ok(());
    };
    let drop_dir = path.with_file_name(format!("{}.d", file_name.to_string_lossy()));
    let mut entries = match std::fs::read_dir(&drop_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|entry| {
                entry
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".conf"))
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(config_error(format!(
                "the client configuration drop-in directory could not be read: {error}"
            )))
        }
    };
    entries.sort();
    for entry in entries {
        match regular_file(&entry) {
            Ok(true) => parse_client_config(&entry, config, stack, files, false)?,
            Ok(false) => {
                return Err(config_error(
                    "a client configuration drop-in is not a regular file",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(config_error(format!(
                    "a client configuration drop-in could not be read: {error}"
                )))
            }
        }
    }
    Ok(())
}

fn parse_config_text(
    path: &Path,
    text: &str,
    config: &mut ClientConfig,
    stack: &mut Vec<PathBuf>,
    files: &mut usize,
) -> Result<(), AudioError> {
    for (index, raw) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(".include ") {
            let value = rest.trim();
            if value.is_empty() {
                return Err(malformed(
                    path,
                    line_number,
                    "an .include directive without a path",
                ));
            }
            let include = if Path::new(value).is_absolute() {
                PathBuf::from(value)
            } else {
                path.parent().unwrap_or_else(|| Path::new(".")).join(value)
            };
            parse_client_config(&include, config, stack, files, false)?;
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') {
                return Err(malformed(path, line_number, "a malformed section header"));
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(malformed(
                path,
                line_number,
                "a configuration line without '='",
            ));
        };
        apply_directive(path, line_number, key.trim(), value.trim(), config)?;
    }
    Ok(())
}

fn apply_directive(
    path: &Path,
    line_number: usize,
    key: &str,
    value: &str,
    config: &mut ClientConfig,
) -> Result<(), AudioError> {
    match key {
        "default-server" => config.default_server = non_empty_text(value),
        "cookie-file" => config.cookie_file = non_empty_text(value).map(PathBuf::from),
        "auto-connect-localhost" | "auto-connect-display" => {
            let enabled = parse_boolean(value).ok_or_else(|| {
                malformed(path, line_number, "an invalid boolean directive value")
            })?;
            if enabled {
                return Err(config_error(format!(
                    "{key} is not supported by the bundled audio client"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

fn read_config_text(path: &Path) -> Result<String, AudioError> {
    if !regular_file(path).map_err(|error| {
        config_error(format!(
            "the client configuration could not be read: {error}"
        ))
    })? {
        return Err(config_error(
            "the client configuration is not a regular file",
        ));
    }
    let file = std::fs::File::open(path).map_err(|error| {
        config_error(format!(
            "the client configuration could not be read: {error}"
        ))
    })?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(config_error(
            "the client configuration is not a regular file",
        ));
    }
    let mut text = String::new();
    let mut reader = file.take(MAX_CONFIG_FILE_BYTES + 1);
    reader.read_to_string(&mut text).map_err(|error| {
        config_error(format!(
            "the client configuration could not be read: {error}"
        ))
    })?;
    if text.len() as u64 > MAX_CONFIG_FILE_BYTES {
        return Err(config_error("the client configuration is too large"));
    }
    Ok(text)
}

fn strip_comment(line: &str) -> &str {
    match line.find(['#', ';']) {
        Some(index) => &line[..index],
        None => line,
    }
}

fn parse_boolean(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "y" | "t" | "yes" | "true" | "on" => Some(true),
        "0" | "n" | "f" | "no" | "false" | "off" => Some(false),
        _ => None,
    }
}

fn non_empty_text(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    Some(value.to_owned())
}

fn regular_file(path: &Path) -> std::io::Result<bool> {
    Ok(std::fs::metadata(path)?.is_file())
}

fn config_error(reason: impl Into<String>) -> AudioError {
    AudioError::Operation(reason.into())
}

fn malformed(path: &Path, line_number: usize, reason: &str) -> AudioError {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| CLIENT_CONFIG_NAME.to_owned());
    AudioError::Operation(format!("{name}:{line_number}: {reason}"))
}

pub(super) fn resolve_socket(
    server: Option<String>,
    runtime: Option<PathBuf>,
    xdg: Option<PathBuf>,
) -> Result<PathBuf, AudioError> {
    let requested = server.as_deref().unwrap_or_default().trim();
    if !requested.is_empty() {
        let endpoint = requested.split_whitespace().next().unwrap_or_default();
        let path = unix_socket_path(endpoint).ok_or_else(|| {
            AudioError::Operation(format!(
                "PULSE_SERVER endpoint '{endpoint}' is not a supported local unix socket"
            ))
        })?;
        if path.exists() {
            return Ok(path);
        }
        return Err(AudioError::ServerUnavailable(
            "the configured audio socket does not exist".to_owned(),
        ));
    }

    if let Some(runtime) = runtime {
        let path = runtime.join("native");
        if path.exists() {
            return Ok(path);
        }
        return Err(AudioError::ServerUnavailable(
            "the PULSE_RUNTIME_PATH socket does not exist".to_owned(),
        ));
    }

    if let Some(xdg) = xdg {
        let path = xdg.join("pulse/native");
        if path.exists() {
            return Ok(path);
        }
        return Err(AudioError::ServerUnavailable(
            "the XDG_RUNTIME_DIR PulseAudio socket does not exist".to_owned(),
        ));
    }

    Err(AudioError::ServerUnavailable(
        "no local PulseAudio socket was found".to_owned(),
    ))
}

fn unix_socket_path(endpoint: &str) -> Option<PathBuf> {
    if let Some(path) = endpoint.strip_prefix("unix:") {
        return Path::new(path).is_absolute().then(|| PathBuf::from(path));
    }
    Path::new(endpoint)
        .is_absolute()
        .then(|| PathBuf::from(endpoint))
}

pub(super) fn resolve_cookie(
    explicit: Option<PathBuf>,
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<[u8; COOKIE_LENGTH], AudioError> {
    let config_dir = config_dir(config_home.as_deref(), home.as_deref());

    if let Some(explicit) = explicit {
        let path = if explicit.is_absolute() {
            explicit
        } else {
            let config_dir = config_dir.ok_or_else(|| {
                AudioError::Authentication(
                    "a relative PULSE_COOKIE path needs a config home directory".to_owned(),
                )
            })?;
            config_dir.join(explicit)
        };
        return read_cookie(&path, "PULSE_COOKIE");
    }

    if let Some(config_dir) = &config_dir {
        let path = config_dir.join("cookie");
        if path.exists() {
            return read_cookie(&path, "the PulseAudio config directory");
        }
    }

    if let Some(home) = &home {
        let path = home.join(".pulse-cookie");
        if path.exists() {
            return read_cookie(&path, "the legacy home cookie");
        }
    }

    Ok([0u8; COOKIE_LENGTH])
}

fn config_dir(config_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(config_home) = config_home {
        return Some(config_home.join("pulse"));
    }
    home.map(|home| home.join(".config/pulse"))
}

fn read_cookie(path: &Path, source: &str) -> Result<[u8; COOKIE_LENGTH], AudioError> {
    if !regular_file(path).map_err(|_| cookie_error(source))? {
        return Err(cookie_error(source));
    }
    let file = std::fs::File::open(path).map_err(|_| cookie_error(source))?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(cookie_error(source));
    }
    let mut bytes = Vec::with_capacity(COOKIE_LENGTH);
    let mut reader = file.take(COOKIE_LENGTH as u64);
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| cookie_error(source))?;
    if bytes.len() != COOKIE_LENGTH {
        return Err(AudioError::Authentication(format!(
            "the cookie from {source} is not {COOKIE_LENGTH} bytes"
        )));
    }
    let mut cookie = [0u8; COOKIE_LENGTH];
    cookie.copy_from_slice(&bytes);
    Ok(cookie)
}

fn cookie_error(source: &str) -> AudioError {
    AudioError::Authentication(format!("the cookie from {source} could not be read"))
}

fn non_empty(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}
