use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BootMedia {
    Disk,
    Iso,
}

impl BootMedia {
    pub(crate) fn from_path(path: &Path) -> BootMedia {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("iso") => BootMedia::Iso,
            _ => BootMedia::Disk,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            BootMedia::Disk => "disk",
            BootMedia::Iso => "iso",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_path_maps_iso_extension_to_iso_else_disk() {
        let cases = [
            ("ubuntu.iso", BootMedia::Iso),
            ("linuxmint-22.1-cinnamon-64bit.iso", BootMedia::Iso),
            ("UPPER.ISO", BootMedia::Iso),
            ("disk.qcow2", BootMedia::Disk),
            ("disk.img", BootMedia::Disk),
            ("disk.vmdk", BootMedia::Disk),
            ("noext", BootMedia::Disk),
        ];
        for (name, expected) in cases {
            assert_eq!(
                BootMedia::from_path(Path::new(name)),
                expected,
                "name: {name}"
            );
        }
    }

    #[test]
    fn as_str_names_each_variant() {
        assert_eq!(BootMedia::Disk.as_str(), "disk");
        assert_eq!(BootMedia::Iso.as_str(), "iso");
    }
}
