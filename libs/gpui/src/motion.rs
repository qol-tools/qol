use std::sync::Mutex;
use std::time::{Duration, Instant};

use gpui::Animation;
use qol_theme::{Animations, Motion};

pub const RECHECK: Duration = Duration::from_secs(2);

pub fn animation(motion: Motion) -> Animation {
    Animation::new(motion.duration).with_easing(move |delta| ease(motion, delta))
}

pub fn after_hold(motion: Motion, hold: Duration) -> Animation {
    Animation::new(hold + motion.duration)
}

pub fn progress(motion: Motion, elapsed: Duration) -> f32 {
    ease(
        motion,
        elapsed.as_secs_f32() / motion.duration.as_secs_f32(),
    )
}

pub fn ease(motion: Motion, delta: f32) -> f32 {
    if moving() {
        motion.curve.at(delta)
    } else {
        1.0
    }
}

pub fn moving() -> bool {
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let Ok(mut cache) = CACHE.lock() else {
        return true;
    };
    if let Some((read_at, moving)) = *cache {
        if read_at.elapsed() < RECHECK {
            return moving;
        }
    }
    let moving = moves(chosen(), crate::platform::reduce_motion);
    *cache = Some((Instant::now(), moving));
    moving
}

fn chosen() -> Animations {
    qol_config::config_dir()
        .and_then(|dir| {
            std::fs::read_to_string(dir.join(qol_conventions::THEME_SETTINGS_FILE)).ok()
        })
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| {
            settings
                .get(qol_conventions::ANIMATIONS_KEY)?
                .as_str()
                .and_then(Animations::parse)
        })
        .unwrap_or_default()
}

fn moves(choice: Animations, system_reduces: impl FnOnce() -> bool) -> bool {
    match choice {
        Animations::On => true,
        Animations::Off => false,
        Animations::System => !system_reduces(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_animations_choice_decides_motion_and_only_follow_system_asks_the_os() {
        let cases = [
            (Animations::On, true, true),
            (Animations::Off, false, false),
            (Animations::System, false, true),
            (Animations::System, true, false),
        ];
        for (choice, system_reduces, expected) in cases {
            let asked = std::cell::Cell::new(false);
            let moving = moves(choice, || {
                asked.set(true);
                system_reduces
            });
            assert_eq!(moving, expected, "{choice:?}");
            assert_eq!(asked.get(), choice == Animations::System, "{choice:?}");
        }
    }
}
