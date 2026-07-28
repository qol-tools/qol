use super::{unix, LifelineAuditPlatform};

pub(super) struct Platform;

impl LifelineAuditPlatform for Platform {
    fn settle_missing_lifelines(expected: &[String]) -> Vec<String> {
        unix::settle_missing_lifelines(expected)
    }
}
