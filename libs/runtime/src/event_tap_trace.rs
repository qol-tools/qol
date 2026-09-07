use std::sync::mpsc::{self, SyncSender};

pub const QUEUE_DEPTH: usize = 256;

pub struct TraceSink {
    lines: SyncSender<String>,
}

impl TraceSink {
    pub fn spawn(
        thread_name: &str,
        depth: usize,
        mut write: impl FnMut(&[String]) + Send + 'static,
    ) -> Self {
        let (lines, incoming) = mpsc::sync_channel::<String>(depth);
        std::thread::Builder::new()
            .name(thread_name.to_owned())
            .spawn(move || {
                while let Ok(line) = incoming.recv() {
                    let mut batch = Vec::with_capacity(4);
                    batch.push(line);
                    while let Ok(more) = incoming.try_recv() {
                        batch.push(more);
                    }
                    write(&batch);
                }
            })
            .expect("the event tap trace writer thread must start");
        Self { lines }
    }
    pub fn offer(&self, line: String) {
        let _ = self.lines.try_send(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    #[test]
    fn a_stalled_writer_never_blocks_the_caller() {
        let (release, blocked) = channel::<()>();
        let (wrote, written) = channel::<String>();
        let sink = TraceSink::spawn("test-tap-trace", QUEUE_DEPTH, move |batch| {
            let _ = blocked.recv();
            let _ = wrote.send(batch.join(""));
        });

        let started = Instant::now();
        for index in 0..QUEUE_DEPTH * 4 {
            sink.offer(format!("line {index}"));
        }
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "the event tap thread must outrun a wedged writer, or macOS kills the tap and the keyboard freezes: {elapsed:?}"
        );
        drop(release);
        assert!(
            written.recv_timeout(Duration::from_secs(5)).is_ok(),
            "lines that fit the queue still reach the writer once it drains"
        );
    }
}
