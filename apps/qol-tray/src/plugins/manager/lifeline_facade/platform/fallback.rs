use super::LifelineAuditPlatform;

pub(super) struct Platform;

impl LifelineAuditPlatform for Platform {
    fn settle_missing_lifelines(_expected: &[String]) -> Vec<String> {
        Vec::new()
    }
}
