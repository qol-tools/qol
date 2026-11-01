use std::sync::OnceLock;

static ACTIVE: OnceLock<bool> = OnceLock::new();

pub fn select(requested: bool) -> bool {
    let reason = reason(requested, qol_platform::has_display());
    if let Some(reason) = reason {
        log::info!("Running headless: {reason}");
    }
    let headless = reason.is_some();
    let _ = ACTIVE.set(headless);
    headless
}

pub fn active() -> bool {
    ACTIVE.get().copied().unwrap_or(false)
}

fn reason(requested: bool, has_display: bool) -> Option<&'static str> {
    if requested {
        Some("--headless was passed")
    } else if !has_display {
        Some("no display was found")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::reason;

    #[test]
    fn headless_follows_the_flag_or_a_missing_display() {
        let cases = [
            (false, true, None),
            (false, false, Some("no display was found")),
            (true, true, Some("--headless was passed")),
            (true, false, Some("--headless was passed")),
        ];
        for (requested, has_display, expected) in cases {
            assert_eq!(
                reason(requested, has_display),
                expected,
                "requested={requested} has_display={has_display}"
            );
        }
    }
}
