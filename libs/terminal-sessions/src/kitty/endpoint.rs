use std::path::{Path, PathBuf};

use super::socket;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Endpoint {
    instance: Option<String>,
    listen_on: Option<String>,
    socket_path: Option<PathBuf>,
}

impl Endpoint {
    pub(super) fn legacy() -> Self {
        Self {
            instance: None,
            listen_on: None,
            socket_path: None,
        }
    }

    pub(super) fn from_path(path: &Path) -> Option<Self> {
        let instance = socket::instance_id(path)?;
        let path_text = path.to_str()?;
        Some(Self {
            instance: Some(instance),
            listen_on: Some(format!("unix:{path_text}")),
            socket_path: Some(path.to_owned()),
        })
    }

    #[cfg(test)]
    pub(super) fn fixture(instance: &str, listen_on: &str) -> Self {
        Self {
            instance: Some(instance.to_owned()),
            listen_on: Some(listen_on.to_owned()),
            socket_path: None,
        }
    }

    pub(super) fn instance(&self) -> Option<&str> {
        self.instance.as_deref()
    }

    pub(super) fn listen_on(&self) -> Option<&str> {
        self.listen_on.as_deref()
    }

    pub(super) fn socket_path(&self) -> Option<&Path> {
        self.socket_path.as_deref()
    }

    pub(super) fn native_id(&self, window_id: u64) -> String {
        match self.instance() {
            Some(instance) => format!("{instance}.{window_id}"),
            None => window_id.to_string(),
        }
    }
}

pub(super) trait EndpointSource: Send + Sync {
    fn endpoints(&self) -> Vec<Endpoint>;
    fn current(&self) -> Endpoint;
}

pub(super) struct SystemEndpointSource;

impl EndpointSource for SystemEndpointSource {
    fn endpoints(&self) -> Vec<Endpoint> {
        let current_path = current_socket_path();
        let mut paths = current_path
            .as_deref()
            .map(socket::discover_sibling_paths)
            .unwrap_or_else(socket::discover_default_paths);
        if let Some(current_path) = current_path.as_ref() {
            if !paths.iter().any(|path| path == current_path) {
                paths.push(current_path.clone());
            }
        }
        paths.sort();
        let mut endpoints = paths
            .into_iter()
            .filter_map(|path| Endpoint::from_path(&path))
            .collect::<Vec<_>>();
        endpoints.sort_by(|left, right| {
            let left_is_current = current_path
                .as_deref()
                .is_some_and(|path| left.socket_path() == Some(path));
            let right_is_current = current_path
                .as_deref()
                .is_some_and(|path| right.socket_path() == Some(path));
            (!left_is_current)
                .cmp(&(!right_is_current))
                .then(left.instance.cmp(&right.instance))
        });
        endpoints.dedup_by(|left, right| left.instance == right.instance);
        if endpoints.is_empty() {
            vec![Endpoint::legacy()]
        } else {
            endpoints
        }
    }

    fn current(&self) -> Endpoint {
        current_socket_path()
            .or_else(|| socket::discover_default_paths().into_iter().next())
            .as_deref()
            .and_then(Endpoint::from_path)
            .unwrap_or_else(Endpoint::legacy)
    }
}

#[cfg(test)]
pub(super) struct LegacyEndpointSource;

#[cfg(test)]
impl EndpointSource for LegacyEndpointSource {
    fn endpoints(&self) -> Vec<Endpoint> {
        vec![Endpoint::legacy()]
    }

    fn current(&self) -> Endpoint {
        Endpoint::legacy()
    }
}

fn current_socket_path() -> Option<PathBuf> {
    socket::connectable_socket_from_listen_on(&std::env::var("KITTY_LISTEN_ON").ok()?)
}
