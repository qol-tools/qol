use std::process::Command;

use anyhow::Result;

pub(crate) fn read(service: &str) -> Result<String> {
    let mut command = Command::new("/usr/bin/security");
    command.args(["find-generic-password", "-s", service, "-w"]);
    super::tool::secret_from(command, service, "the login Keychain")
}
