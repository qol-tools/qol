mod platform;

pub(super) fn settle_missing_lifelines(expected: &[String]) -> Vec<String> {
    platform::settle_missing_lifelines(expected)
}
