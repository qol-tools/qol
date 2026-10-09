use std::io::{self, Read};

use windows_sys::Win32::System::Console::SetConsoleTitleW;

use super::attach::Detached;

pub(super) fn run(args: &[String]) -> io::Result<String> {
    let [pid] = args else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a process id",
        ));
    };
    let pid = pid.parse::<u32>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid process id {pid:?}"),
        )
    })?;
    let mut title = String::new();
    io::stdin().read_to_string(&mut title)?;
    let scope = Detached::begin();
    let _attached = scope.attach(pid)?;
    set(&title)?;
    Ok(String::new())
}

pub(super) fn set(title: &str) -> io::Result<()> {
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    if unsafe { SetConsoleTitleW(wide.as_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
