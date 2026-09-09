use std::borrow::Cow;

use gpui::App;

pub(crate) const SANS_MEDIUM: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf");

const FACES: [&[u8]; 7] = [
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    SANS_MEDIUM,
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Bold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
];

pub fn install(cx: &App) {
    let faces = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    match cx.text_system().add_fonts(faces) {
        Ok(()) => qol_runtime::probe!("GPUI_FONTS", "phase=installed faces={}", FACES.len()),
        Err(error) => qol_runtime::probe!("GPUI_FONTS", "phase=install-failed error={error}"),
    }
}
