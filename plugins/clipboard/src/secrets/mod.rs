use std::time::Duration;

pub const SENSITIVE_FORMATS: [&str; 3] = [
    "x-kde-passwordManagerHint",
    "ExcludeClipboardContentFromMonitorProcessing",
    "org.nspasteboard.ConcealedType",
];

const MASK: &str = "••••••••";

pub fn flagged_by_source<S: AsRef<str>>(formats: &[S]) -> bool {
    formats
        .iter()
        .any(|format| SENSITIVE_FORMATS.contains(&format.as_ref()))
}

pub fn mask() -> &'static str {
    MASK
}

pub fn remaining(copied_at_ms: u64, now_ms: u64, ttl: Duration) -> Option<Duration> {
    let deadline = copied_at_ms.saturating_add(ttl.as_millis() as u64);
    (deadline > now_ms).then(|| Duration::from_millis(deadline - now_ms))
}

pub fn expired(copied_at_ms: u64, now_ms: u64, ttl: Duration) -> bool {
    remaining(copied_at_ms, now_ms, ttl).is_none()
}

pub fn countdown(remaining: Duration) -> String {
    let seconds = remaining.as_millis().div_ceil(1000);
    format!("clears in {seconds} s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_markers_flag_a_copy_as_secret() {
        let cases: &[(&[&str], bool)] = &[
            (
                &["TARGETS", "UTF8_STRING", "x-kde-passwordManagerHint"],
                true,
            ),
            (&["ExcludeClipboardContentFromMonitorProcessing"], true),
            (
                &["public.utf8-plain-text", "org.nspasteboard.ConcealedType"],
                true,
            ),
            (&["TARGETS", "UTF8_STRING", "text/plain"], false),
            (&[], false),
        ];
        for (formats, flagged) in cases {
            assert_eq!(flagged_by_source(formats), *flagged, "formats: {formats:?}");
        }
    }

    #[test]
    fn secrets_expire_after_the_ttl() {
        let ttl = Duration::from_secs(30);
        let copied = 1_000_000;
        let cases = [
            (copied, Some(Duration::from_secs(30))),
            (copied + 8_000, Some(Duration::from_secs(22))),
            (copied + 29_999, Some(Duration::from_millis(1))),
            (copied + 30_000, None),
            (copied + 90_000, None),
        ];
        for (now, left) in cases {
            assert_eq!(remaining(copied, now, ttl), left, "now: {now}");
            assert_eq!(expired(copied, now, ttl), left.is_none(), "now: {now}");
        }
    }

    #[test]
    fn countdown_rounds_up_to_whole_seconds() {
        let cases = [
            (Duration::from_secs(22), "clears in 22 s"),
            (Duration::from_millis(21_001), "clears in 22 s"),
            (Duration::from_millis(1), "clears in 1 s"),
        ];
        for (left, label) in cases {
            assert_eq!(countdown(left), label);
        }
    }

    #[test]
    fn mask_has_a_fixed_width() {
        assert_eq!(mask().chars().count(), 8);
    }
}
