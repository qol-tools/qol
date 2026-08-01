use std::hash::Hasher;
use std::path::PathBuf;
use std::sync::Arc;

use super::inputs::FingerprintInput;

pub(super) type FingerprintContent = (PathBuf, Arc<[u8]>);

pub(super) fn read_inputs(
    inputs: Vec<FingerprintInput>,
) -> Result<Vec<FingerprintContent>, String> {
    inputs
        .into_iter()
        .map(|(relative_path, absolute_path)| {
            let contents = std::fs::read(&absolute_path).map_err(|error| {
                format!("Failed to read {}: {}", absolute_path.display(), error)
            })?;
            Ok((relative_path, Arc::<[u8]>::from(contents)))
        })
        .collect()
}

pub(super) fn hash_contents(mut inputs: Vec<FingerprintContent>) -> Result<String, String> {
    inputs.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut hasher = Fnv1a64::default();
    for (relative_path, contents) in inputs {
        hash_path(&mut hasher, &relative_path);
        hasher.write(&contents);
        hasher.write_u8(0xff);
    }

    Ok(format!("{:016x}", hasher.finish()))
}

fn hash_path(hasher: &mut Fnv1a64, relative_path: &std::path::Path) {
    hasher.write(relative_path.to_string_lossy().as_bytes());
    hasher.write_u8(0);
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
