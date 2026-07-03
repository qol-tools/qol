#[derive(Clone)]
pub(crate) enum LiveFrame {}

pub(crate) fn live_frame_element(frame: &LiveFrame, _width: f32, _height: f32) -> gpui::AnyElement {
    match *frame {}
}

#[derive(Clone)]
pub(crate) struct SendCVBuf(LiveFrame);

impl SendCVBuf {
    pub(crate) fn pixel_format(&self) -> u32 {
        match self.0 {}
    }

    pub(crate) fn into_live_frame(self) -> LiveFrame {
        match self.0 {}
    }
}

pub(crate) type ShotReply = (u32, Option<SendCVBuf>);

pub(crate) struct ShotsSession;

impl ShotsSession {
    pub(crate) fn request_capture(
        &self,
        _wid: u32,
        _max_w: usize,
        _max_h: usize,
        _reply: &std::sync::mpsc::Sender<ShotReply>,
    ) -> bool {
        false
    }
}

pub(crate) const PIXEL_FORMAT_420F: u32 = 0x3432_3066;

pub(crate) fn live_shots_available() -> bool {
    false
}

pub(crate) fn fetch_shots_session() -> Option<ShotsSession> {
    None
}

pub(crate) fn cached_shots_session(_required: &[u32]) -> Option<std::sync::Arc<ShotsSession>> {
    None
}

pub(crate) fn warm_shots_session(_required: &[u32]) -> Option<std::sync::Arc<ShotsSession>> {
    None
}
