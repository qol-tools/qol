use super::DaemonLifecyclePlatform;
use std::os::unix::process::CommandExt;
use std::process::Command;

pub(super) struct Platform;

impl DaemonLifecyclePlatform for Platform {
    fn reaped_elsewhere(error: &std::io::Error) -> bool {
        error.raw_os_error() == Some(libc::ECHILD)
    }

    fn configure_process_group(command: &mut Command) {
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
}
