#![allow(clippy::print_stdout)]

/// Prints one `x,y,w,h` work area per line for the `SCREENS_ACTION` subcommand.
pub(super) fn print_work_areas() {
    for rect in super::screen::system_screens() {
        println!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h);
    }
}
