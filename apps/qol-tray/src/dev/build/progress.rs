use std::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub(crate) struct CargoProgressSnapshot {
    pub done: u32,
    pub total: u32,
    pub phase: String,
}

#[derive(Debug, Default)]
pub(crate) struct CargoProgressEstimator {
    baseline_done: Option<u32>,
    last_done: u32,
    last_elapsed_secs: f64,
    avg_unit_secs: Option<f64>,
    ratio: f64,
}

impl CargoProgressEstimator {
    pub(crate) fn update(
        &mut self,
        observed_done: u32,
        observed_total: u32,
        elapsed_secs: f64,
    ) -> (u8, u32, u32) {
        if self.baseline_done.is_none() {
            if observed_done == 0 {
                self.last_elapsed_secs = elapsed_secs;
                return (0, 0, observed_total.max(1));
            }

            // Cargo can emit an initial 0/N snapshot and then jump to high done counts
            // for already-cached units. Rebase to the first non-zero observation so this
            // run reflects only work still outstanding.
            let rebased = observed_done
                .saturating_sub(1)
                .min(observed_total.saturating_sub(1));
            self.baseline_done = Some(rebased);
        }

        let baseline = self.baseline_done.unwrap_or(0);

        let mut total = observed_total.saturating_sub(baseline);
        if total == 0 {
            total = 1;
        }
        let done = observed_done.saturating_sub(baseline).min(total);

        let delta_done = done.saturating_sub(self.last_done);
        let delta_elapsed = (elapsed_secs - self.last_elapsed_secs).max(0.0);
        if delta_done > 0 && delta_elapsed > 0.0 {
            let sample_unit_secs = delta_elapsed / delta_done as f64;
            self.avg_unit_secs = Some(match self.avg_unit_secs {
                Some(previous) => (previous * 0.7) + (sample_unit_secs * 0.3),
                None => sample_unit_secs,
            });
        }

        self.last_done = done;
        self.last_elapsed_secs = elapsed_secs;

        let raw_ratio = (done as f64 / total as f64).clamp(0.0, 1.0);
        let time_ratio = if let Some(avg_unit_secs) = self.avg_unit_secs {
            let remaining_units = total.saturating_sub(done) as f64;
            if remaining_units > 0.0 {
                let eta_secs = avg_unit_secs * remaining_units;
                if eta_secs > 0.0 {
                    (elapsed_secs / (elapsed_secs + eta_secs)).clamp(0.0, 1.0)
                } else {
                    raw_ratio
                }
            } else {
                1.0
            }
        } else {
            raw_ratio
        };

        let mut ratio = (raw_ratio * 0.35) + (time_ratio * 0.65);
        if done < total {
            ratio = ratio.min(0.985);
        } else {
            ratio = 0.99;
        }

        // Keep progress monotonic even when ETA expands after slower late-stage crates.
        ratio = ratio.max(self.ratio);
        self.ratio = ratio;

        let mut percent = (ratio * 99.0).round() as u8;
        if done > 0 {
            percent = percent.max(1);
        }
        (percent.min(99), done, total)
    }
}

pub(crate) fn drain_console_segments(pending: &mut String, mut on_segment: impl FnMut(&str)) {
    while let Some(idx) = pending.find(|c| c == '\n' || c == '\r') {
        let segment = pending[..idx].to_string();
        pending.drain(..=idx);
        on_segment(&segment);
    }
}

pub(crate) fn handle_cargo_console_segment(
    raw_segment: &str,
    progress_tx: &Sender<CargoProgressSnapshot>,
    text_tx: &Sender<String>,
) {
    let line = sanitize_console_line(raw_segment);
    if line.is_empty() {
        return;
    }

    if let Some((done, total, phase)) = parse_cargo_progress_line(&line) {
        let _ = progress_tx.send(CargoProgressSnapshot { done, total, phase });
    }

    let _ = text_tx.send(line);
}

pub(crate) fn sanitize_console_line(raw: &str) -> String {
    let mut sanitized = String::with_capacity(raw.len());
    #[derive(Copy, Clone)]
    enum AnsiState {
        None,
        Escape,
        Csi,
    }
    let mut state = AnsiState::None;

    for ch in raw.chars() {
        match state {
            AnsiState::None => {
                if ch == '\u{1b}' {
                    state = AnsiState::Escape;
                } else if !ch.is_control() {
                    sanitized.push(ch);
                }
            }
            AnsiState::Escape => {
                if ch == '[' {
                    state = AnsiState::Csi;
                } else if ('@'..='~').contains(&ch) {
                    state = AnsiState::None;
                }
            }
            AnsiState::Csi => {
                if ('@'..='~').contains(&ch) {
                    state = AnsiState::None;
                }
            }
        }
    }

    sanitized.trim().to_string()
}

pub(crate) fn parse_cargo_progress_line(line: &str) -> Option<(u32, u32, String)> {
    if !line.contains("Building [") {
        return None;
    }

    let bar_end = line.rfind(']')?;
    let tail = line.get(bar_end + 1..)?.trim();

    let mut tail_parts = tail.splitn(2, ':');
    let ratio = tail_parts.next()?.trim();
    let phase = tail_parts.next().unwrap_or("").trim().to_string();

    let mut ratio_parts = ratio.split('/');
    let done = ratio_parts.next()?.trim().parse::<u32>().ok()?;
    let total = ratio_parts.next()?.trim().parse::<u32>().ok()?;

    if total == 0 || done > total {
        return None;
    }

    Some((done, total, phase))
}
