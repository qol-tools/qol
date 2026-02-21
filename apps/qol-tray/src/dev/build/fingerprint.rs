use std::ffi::OsStr;
use std::hash::Hasher;
use std::io::Read;
use std::path::Path;
use walkdir::WalkDir;

pub(crate) fn fingerprint_plugin(path: &Path) -> Result<String, String> {
    let mut hasher = Fnv1a64::default();
    let mut inputs = Vec::new();

    let walker = WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            !(entry.file_type().is_dir() && should_skip_dir(entry.file_name()))
        });

    for entry in walker {
        let entry = entry.map_err(|e| format!("Walk error: {}", e))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative_path = entry
            .path()
            .strip_prefix(path)
            .map_err(|e| format!("Failed to relativize path: {}", e))?;

        if !is_fingerprint_input(relative_path) {
            continue;
        }
        inputs.push((relative_path.to_path_buf(), entry.path().to_path_buf()));
    }

    if inputs.is_empty() {
        return Err("No Rust build inputs found".to_string());
    }

    inputs.sort_by(|(left, _), (right, _)| left.cmp(right));

    for (relative_path, absolute_path) in inputs {
        hasher.write(relative_path.to_string_lossy().as_bytes());
        hasher.write_u8(0);

        let mut file = std::fs::File::open(&absolute_path)
            .map_err(|e| format!("Failed to open {}: {}", absolute_path.display(), e))?;
        let mut buf = [0u8; 8192];
        loop {
            let read = file
                .read(&mut buf)
                .map_err(|e| format!("Failed to read {}: {}", absolute_path.display(), e))?;
            if read == 0 {
                break;
            }
            hasher.write(&buf[..read]);
        }
        hasher.write_u8(0xff);
    }

    Ok(format!("{:016x}", hasher.finish()))
}

fn should_skip_dir(name: &OsStr) -> bool {
    matches!(name.to_str(), Some("target" | ".git" | ".hg" | ".svn"))
}

fn is_fingerprint_input(relative_path: &Path) -> bool {
    let Some(file_name) = relative_path.file_name().and_then(OsStr::to_str) else {
        return false;
    };

    if matches!(
        file_name,
        "Cargo.toml" | "Cargo.lock" | "build.rs" | "rust-toolchain" | "rust-toolchain.toml"
    ) {
        return true;
    }

    if relative_path
        .components()
        .any(|component| component.as_os_str() == OsStr::new(".cargo"))
    {
        return true;
    }

    relative_path
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("rs"))
        .unwrap_or(false)
}

#[derive(Default)]
struct Fnv1a64(u64);

impl Hasher for Fnv1a64 {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= *byte as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}
