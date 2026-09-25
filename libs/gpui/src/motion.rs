use gpui::Animation;
use qol_theme::Motion;

pub fn animation(motion: Motion) -> Animation {
    Animation::new(motion.duration).with_easing(move |delta| motion.curve.at(delta))
}
