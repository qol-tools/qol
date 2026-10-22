use std::borrow::Cow;
use std::time::{Duration, Instant};

use gpui::{font, px, App, Font, FontWeight, LineFragment};
use qol_theme::{Face, TextStyle};

pub(crate) const SANS_MEDIUM: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf");

pub(crate) const FACES: [&[u8]; 8] = [
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    SANS_MEDIUM,
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Bold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/SairaSemiCondensed-SemiBold.ttf"),
];

const EMOJI_FALLBACK_CODE_POINT: u32 = 0x1F389;
const FALLBACK_WARMUP_DELAY: Duration = Duration::from_millis(500);

pub fn install(cx: &App) {
    let faces = FACES.iter().map(|face| Cow::Borrowed(*face)).collect();
    match cx.text_system().add_fonts(faces) {
        Ok(()) => qol_runtime::probe!("GPUI_FONTS", "phase=installed faces={}", FACES.len()),
        Err(error) => qol_runtime::probe!("GPUI_FONTS", "phase=install-failed error={error}"),
    }
    warm_fallback_fonts(cx);
}

fn warm_fallback_fonts(cx: &App) {
    let text_system = cx.text_system().clone();
    let executor = cx.background_executor().clone();
    cx.background_executor()
        .spawn(async move {
            executor.timer(FALLBACK_WARMUP_DELAY).await;
            let started = Instant::now();
            let faces = styled_faces();
            let emoji = char::from_u32(EMOJI_FALLBACK_CODE_POINT)
                .map(String::from)
                .unwrap_or_default();
            for (face, weight) in &faces {
                let styled = Font {
                    weight: FontWeight(f32::from(*weight)),
                    ..font(face.family())
                };
                let mut wrapper = text_system.line_wrapper(styled, px(14.0));
                let fragments = [LineFragment::text(&emoji)];
                wrapper.wrap_line(&fragments, px(f32::MAX)).for_each(drop);
            }
            qol_runtime::probe!(
                "GPUI_FONTS",
                "phase=fallback-warmed faces={} ms={}",
                faces.len(),
                started.elapsed().as_millis()
            );
        })
        .detach();
}

fn styled_faces() -> Vec<(Face, u16)> {
    let mut faces = Vec::new();
    for style in TextStyle::ALL {
        let spec = style.spec();
        if !faces.contains(&(spec.face, spec.weight)) {
            faces.push((spec.face, spec.weight));
        }
    }
    faces
}
