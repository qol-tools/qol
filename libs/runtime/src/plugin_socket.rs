use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::local_ipc;

const SOCKETS_SUBDIR: &str = "sockets";
const FALLBACK_SOCKET_NAME: &str = "qol-plugin.sock";

pub fn runtime_sockets_dir() -> Option<PathBuf> {
    qol_config::runtime_dir().map(|path| path.join(SOCKETS_SUBDIR))
}

pub fn assigned_socket_path_in(runtime_dir: &Path, socket_name: &str) -> PathBuf {
    local_ipc::socket_path(
        &runtime_dir.join(SOCKETS_SUBDIR),
        socket_file_name(socket_name),
    )
}

pub fn assigned_socket_path(socket_name: &str) -> Option<PathBuf> {
    runtime_sockets_dir().map(|dir| local_ipc::socket_path(&dir, socket_file_name(socket_name)))
}

fn socket_file_name(socket_name: &str) -> &OsStr {
    Path::new(socket_name)
        .file_name()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| OsStr::new(FALLBACK_SOCKET_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_socket_path_in_strips_the_declared_prefix() {
        let cases = [
            ("/tmp/qol-keyremap.sock", "qol-keyremap.sock"),
            ("qol-launcher.sock", "qol-launcher.sock"),
            ("/var/run/qol/custom.sock", "custom.sock"),
        ];
        let runtime = PathBuf::from("/custom/runtime");
        for (declared, expected) in cases {
            let path = assigned_socket_path_in(&runtime, declared);
            assert_eq!(
                path,
                runtime.join(SOCKETS_SUBDIR).join(expected),
                "declared={declared}"
            );
        }
    }

    #[test]
    fn an_empty_or_root_socket_name_falls_back_to_the_default() {
        let runtime = PathBuf::from("/base");
        for declared in ["", "/"] {
            let path = assigned_socket_path_in(&runtime, declared);
            assert_eq!(
                path,
                runtime.join(SOCKETS_SUBDIR).join(FALLBACK_SOCKET_NAME),
                "declared={declared:?}"
            );
        }
    }
}
