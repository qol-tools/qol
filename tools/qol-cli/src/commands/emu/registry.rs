use anyhow::{anyhow, Result};

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QemuImgInfo {
    pub(crate) format: String,
    pub(crate) virtual_size: u64,
}

const KNOWN_FORMATS: &[&str] = &["qcow2", "qcow", "raw", "vhd", "vhdx", "vmdk", "vpc"];

#[allow(dead_code)]
pub(crate) fn parse_qemu_img_info(json: &str) -> Result<QemuImgInfo> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| anyhow!("invalid qemu-img JSON: {e}"))?;
    let format = value
        .get("format")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("qemu-img info missing `format`"))?
        .to_string();
    if !KNOWN_FORMATS.contains(&format.as_str()) {
        return Err(anyhow!("unknown image format `{format}`"));
    }
    let virtual_size = value
        .get("virtual-size")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("qemu-img info missing `virtual-size`"))?;
    Ok(QemuImgInfo {
        format,
        virtual_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qemu_img_info_json() {
        let json = r#"{"virtual-size":21474836480,"filename":"/a/b/x.qcow2","format":"qcow2","actual-size":1234}"#;
        let info = parse_qemu_img_info(json).unwrap();
        assert_eq!(info.format, "qcow2");
        assert_eq!(info.virtual_size, 21474836480);
    }

    #[test]
    fn rejects_missing_format() {
        let json = r#"{"virtual-size":1024}"#;
        let error = parse_qemu_img_info(json).unwrap_err();
        assert!(error.to_string().contains("format"), "error: {error}");
    }

    #[test]
    fn rejects_unknown_format() {
        let json = r#"{"format":"mystery","virtual-size":1024}"#;
        let error = parse_qemu_img_info(json).unwrap_err();
        assert!(
            error.to_string().contains("unknown image format"),
            "error: {error}"
        );
    }
}
