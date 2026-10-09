use anyhow::{bail, Context, Result};

pub(crate) fn device_id() -> Result<String> {
    let raw = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .context("failed to run ioreg for the residency device identity")?;
    if !raw.status.success() {
        bail!("ioreg failed to enumerate the platform device");
    }
    let output = String::from_utf8_lossy(&raw.stdout);
    let uuid = parse_platform_uuid(&output).context("failed to parse IOPlatformUUID")?;
    Ok(derive_device_id(&uuid))
}

fn derive_device_id(source: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update("residency".as_bytes());
    hasher.update(b":");
    hasher.update(source.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("qol-resident-{}", &digest[..16])
}

fn parse_platform_uuid(output: &str) -> Result<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("IOPlatformUUID") {
            continue;
        }
        let Some(rhs) = trimmed.split('=').nth(1) else {
            continue;
        };
        let uuid = rhs.trim().trim_matches('"').trim();
        if !uuid.is_empty() {
            return Ok(uuid.to_string());
        }
    }
    bail!("no IOPlatformUUID found in the ioreg output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_platform_uuid_parser_extracts_a_quoted_uuid() {
        let output = "    \"IOPlatformUUID\" = \"AAAABBBB-CCCC-DDDD-EEEE-FFFF0099AABB\"\n";
        assert_eq!(
            parse_platform_uuid(output).unwrap(),
            "AAAABBBB-CCCC-DDDD-EEEE-FFFF0099AABB"
        );
    }

    #[test]
    fn macos_platform_uuid_parser_rejects_missing_or_empty_values() {
        let missing = "    \"IOPlatformName\" = \"Mac\"\n";
        assert!(parse_platform_uuid(missing).is_err());
    }

    #[test]
    fn macos_device_id_is_machine_derived_and_stable_in_shape() {
        let id = derive_device_id("AAAABBBB-CCCC-DDDD-EEEE-FFFF0099AABB");
        assert!(id.starts_with("qol-resident-"), "{id}");
        assert_eq!(id.len(), "qol-resident-".len() + 16, "{id}");
    }
}
