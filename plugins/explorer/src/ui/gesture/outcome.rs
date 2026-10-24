use super::hold::Phase;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Ignore,
    Swallow,
    Begin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Keep,
    Clear,
    Restore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Keeps,
    Gone,
    OffScreen,
}

pub fn start(closed_menu: bool, eligible: bool) -> Start {
    if closed_menu {
        Start::Swallow
    } else if eligible {
        Start::Begin
    } else {
        Start::Ignore
    }
}

pub fn release(phase: Phase, on_row: bool, additive: bool) -> Release {
    match phase {
        Phase::Lifted => Release::Restore,
        Phase::Pending if !on_row && !additive => Release::Clear,
        Phase::Pending | Phase::Boxing => Release::Keep,
    }
}

pub fn restores_on_cancel(phase: Phase) -> bool {
    matches!(phase, Phase::Boxing | Phase::Lifted)
}

pub fn carries_on_hand_over(phase: Phase) -> bool {
    phase == Phase::Lifted
}

pub fn standing(top: Option<f32>, pitch: f32, scroll: f32, height: f32) -> Standing {
    match top {
        None => Standing::Gone,
        Some(top) if top + pitch > scroll && top < scroll + height => Standing::Keeps,
        Some(_) => Standing::OffScreen,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        carries_on_hand_over, release, restores_on_cancel, standing, start, Release, Standing,
        Start,
    };
    use crate::ui::gesture::Phase;

    #[test]
    fn a_press_that_closed_the_menu_is_swallowed_and_never_begins() {
        let cases = [
            (true, true, Start::Swallow),
            (true, false, Start::Swallow),
            (false, true, Start::Begin),
            (false, false, Start::Ignore),
        ];
        for (closed_menu, eligible, want) in cases {
            assert_eq!(
                start(closed_menu, eligible),
                want,
                "{closed_menu} {eligible}"
            );
        }
    }

    #[test]
    fn releasing_a_lift_restores_the_picks_and_a_plain_click_on_nothing_clears_them() {
        let cases = [
            (Phase::Pending, true, false, Release::Keep),
            (Phase::Pending, true, true, Release::Keep),
            (Phase::Pending, false, false, Release::Clear),
            (Phase::Pending, false, true, Release::Keep),
            (Phase::Boxing, true, false, Release::Keep),
            (Phase::Boxing, true, true, Release::Keep),
            (Phase::Boxing, false, false, Release::Keep),
            (Phase::Boxing, false, true, Release::Keep),
            (Phase::Lifted, true, false, Release::Restore),
            (Phase::Lifted, true, true, Release::Restore),
            (Phase::Lifted, false, false, Release::Restore),
            (Phase::Lifted, false, true, Release::Restore),
        ];
        for (phase, on_row, additive, want) in cases {
            assert_eq!(
                release(phase, on_row, additive),
                want,
                "{phase:?} row {on_row} additive {additive}"
            );
        }
    }

    #[test]
    fn escape_restores_the_picks_of_a_box_and_of_a_lift_but_not_of_a_wait() {
        let cases = [
            (Phase::Pending, false),
            (Phase::Boxing, true),
            (Phase::Lifted, true),
        ];
        for (phase, want) in cases {
            assert_eq!(restores_on_cancel(phase), want, "{phase:?}");
        }
    }

    #[test]
    fn only_a_lift_hands_its_carry_to_the_drag() {
        let cases = [
            (Phase::Pending, false),
            (Phase::Boxing, false),
            (Phase::Lifted, true),
        ];
        for (phase, want) in cases {
            assert_eq!(carries_on_hand_over(phase), want, "{phase:?}");
        }
    }

    #[test]
    fn a_lifted_row_is_kept_only_while_it_exists_and_any_part_of_it_is_on_screen() {
        let cases = [
            (None, 0.0, Standing::Gone),
            (None, 5000.0, Standing::Gone),
            (Some(0.0), 0.0, Standing::Keeps),
            (Some(0.0), 29.0, Standing::Keeps),
            (Some(0.0), 30.0, Standing::OffScreen),
            (Some(0.0), 31.0, Standing::OffScreen),
            (Some(300.0), 0.0, Standing::Keeps),
            (Some(399.0), 0.0, Standing::Keeps),
            (Some(400.0), 0.0, Standing::OffScreen),
            (Some(401.0), 0.0, Standing::OffScreen),
            (Some(999.0), 600.0, Standing::Keeps),
            (Some(1000.0), 600.0, Standing::OffScreen),
            (Some(571.0), 600.0, Standing::Keeps),
            (Some(570.0), 600.0, Standing::OffScreen),
            (Some(100.0), 600.0, Standing::OffScreen),
        ];
        for (top, scroll, want) in cases {
            assert_eq!(standing(top, 30.0, scroll, 400.0), want, "{top:?} {scroll}");
        }
    }
}
