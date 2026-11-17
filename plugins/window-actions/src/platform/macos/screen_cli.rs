#![allow(clippy::print_stdout)]

pub(super) fn print_work_areas() {
    for rect in super::screen::system_screens() {
        println!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h);
    }
}
