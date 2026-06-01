use std::hash::Hasher;
use std::io::Read;
use std::path::PathBuf;

use super::inputs::FingerprintInput;

pub(super) fn hash_inputs(mut inputs: Vec<FingerprintInput>) -> Result<String, String> {
    inputs.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut hasher = Fnv1a64::default();
    for (relative_path, absolute_path) in inputs {
        hash_path(&mut hasher, &relative_path);
        hash_file(&mut hasher, &absolute_path)?;
        hasher.write_u8(0xff);
    }

    Ok(format!("{:016x}", hasher.finish()))
}

fn hash_path(hasher: &mut Fnv1a64, relative_path: &std::path::Path) {
    hasher.write(relative_path.to_string_lossy().as_bytes());
    hasher.write_u8(0);
}

fn hash_file(hasher: &mut Fnv1a64, absolute_path: &PathBuf) -> Result<(), String> {
    let mut file = std::fs::File::open(absolute_path)
        .map_err(|error| format!("Failed to open {}: {}", absolute_path.display(), error))?;
    let mut buf = [0u8; 8192];

    loop {
        let read = file
            .read(&mut buf)
            .map_err(|error| format!("Failed to read {}: {}", absolute_path.display(), error))?;
        if read == 0 {
            return Ok(());
        }
        hasher.write(&buf[..read]);
    }
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
