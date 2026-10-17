#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fmt::Display;
use std::io;

pub(crate) fn line(text: impl Display) {
    println!("{text}");
}

pub(crate) fn text(text: impl Display) {
    print!("{text}");
}

pub(crate) fn blank() {
    println!();
}

pub(crate) fn warn(text: impl Display) {
    eprintln!("{text}");
}

pub(crate) fn stdout() -> io::Stdout {
    io::stdout()
}
