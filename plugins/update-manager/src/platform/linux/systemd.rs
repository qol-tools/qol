#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unit {
    pub(crate) loaded: bool,
    pub(crate) enabled: bool,
}

pub(crate) fn unit(shown: &str) -> Unit {
    let mut unit = Unit::default();
    for (key, value) in shown.lines().filter_map(|line| line.split_once('=')) {
        match key {
            "LoadState" => unit.loaded = value == "loaded",
            "UnitFileState" => unit.enabled = value == "enabled",
            _ => {}
        }
    }
    unit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_is_loaded_and_enabled_only_when_systemctl_says_so() {
        assert_eq!(
            unit("LoadState=loaded\nUnitFileState=enabled\n"),
            Unit {
                loaded: true,
                enabled: true
            }
        );
        assert_eq!(
            unit("LoadState=loaded\nUnitFileState=disabled\n"),
            Unit {
                loaded: true,
                enabled: false
            }
        );
        assert_eq!(
            unit("LoadState=not-found\nUnitFileState=\n"),
            Unit::default()
        );
    }
}
