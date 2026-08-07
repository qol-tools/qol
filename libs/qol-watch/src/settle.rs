use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use super::{watch, Watch, WatchError, WatchNotice, WatchRoot};

pub fn settled(
    roots: &[WatchRoot],
    quiet: Duration,
) -> Result<(Watch, Receiver<Vec<PathBuf>>), WatchError> {
    let (raw, incoming) = mpsc::channel();
    let watch = watch(roots, move |notice| {
        if let WatchNotice::Changed(paths) = notice {
            let _ = raw.send(paths);
        }
    })?;
    let (settled, batches) = mpsc::channel();
    thread::Builder::new()
        .name("qol-watch-settle".to_owned())
        .spawn(move || coalesce(&incoming, &settled, quiet))
        .map_err(|error| WatchError::NoWatchableRoot(error.to_string()))?;
    Ok((watch, batches))
}

fn coalesce(
    incoming: &Receiver<Vec<PathBuf>>,
    settled: &mpsc::Sender<Vec<PathBuf>>,
    quiet: Duration,
) {
    let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
    loop {
        let received = if pending.is_empty() {
            incoming.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            incoming.recv_timeout(quiet)
        };
        match received {
            Ok(paths) => pending.extend(paths),
            Err(RecvTimeoutError::Timeout) => {
                if settled
                    .send(std::mem::take(&mut pending).into_iter().collect())
                    .is_err()
                {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !pending.is_empty() {
                    let _ = settled.send(pending.into_iter().collect());
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::settled;
    use crate::WatchRoot;

    #[test]
    fn a_burst_of_changes_arrives_as_one_batch() {
        let root = TempDir::new().unwrap();
        let (_watch, batches) =
            settled(&[WatchRoot::deep(root.path())], Duration::from_millis(200)).unwrap();

        let model = root.path().join("parakeet");
        fs::create_dir(&model).unwrap();
        for name in ["encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"] {
            fs::write(model.join(name), b"weights").unwrap();
        }

        let batch = batches.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            batch.iter().any(|path| path.ends_with("tokens.txt")),
            "the settled batch must carry the files that landed: {batch:?}"
        );
        assert!(
            batches.recv_timeout(Duration::from_millis(400)).is_err(),
            "a settled burst must not keep emitting"
        );
    }
}
