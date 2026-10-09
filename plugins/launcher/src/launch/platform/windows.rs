use std::ffi::OsStr;
use std::io;
use std::path::Path;

pub(crate) fn daemon_action_args(_path: &Path, exec: &[String]) -> Option<(String, String)> {
    super::daemon_exec_args(exec).map(|(target, action)| (target.to_string(), action.to_string()))
}

pub(crate) fn launch_app(_path: &Path, exec: &[String]) -> io::Result<()> {
    let Some((target, args)) = exec.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "application has no executable",
        ));
    };
    qol_apps::shell_execute::shell_execute(OsStr::new(target), args)
}
