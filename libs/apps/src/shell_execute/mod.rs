use std::ffi::OsStr;
use std::io;

mod platform;

pub fn shell_execute(target: &OsStr, args: &[String]) -> io::Result<()> {
    platform::shell_execute(target, args)
}
