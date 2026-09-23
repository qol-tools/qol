#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformSupport {
    pub(crate) name: &'static str,
    pub(crate) supported: bool,
}
