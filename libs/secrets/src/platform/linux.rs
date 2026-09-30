use std::process::Command;

use anyhow::Result;

pub(crate) fn read(service: &str) -> Result<String> {
    let mut command = Command::new("secret-tool");
    command.args(["lookup", "service", service]);
    super::tool::secret_from(
        command,
        service,
        "the Secret Service (secret-tool, from libsecret-tools)",
    )
}
