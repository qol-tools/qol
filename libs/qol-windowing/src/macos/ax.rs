pub fn is_normal_window_subrole(subrole: Option<&str>) -> bool {
    matches!(subrole, None | Some("AXStandardWindow" | "AXDialog"))
}

#[cfg(test)]
mod tests {
    use super::is_normal_window_subrole;

    #[test]
    fn normal_subroles_are_accepted() {
        assert!(is_normal_window_subrole(Some("AXStandardWindow")));
        assert!(is_normal_window_subrole(Some("AXDialog")));
    }

    #[test]
    fn unreadable_or_missing_subrole_is_normal() {
        assert!(is_normal_window_subrole(None));
    }

    #[test]
    fn floating_and_overlay_subroles_are_not_normal() {
        for subrole in ["AXFloatingWindow", "AXUnknown", "AXSheet", "", "AXPanel"] {
            assert!(!is_normal_window_subrole(Some(subrole)), "{subrole:?}");
        }
    }
}
