use anyhow::{bail, Result};

pub(super) mod x11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DisplayServer {
    X11,
    Wayland,
    Unknown,
}

fn from_session_type(value: Option<&str>) -> DisplayServer {
    let Some(value) = value.map(str::trim) else {
        return DisplayServer::Unknown;
    };
    if value.eq_ignore_ascii_case("wayland") {
        DisplayServer::Wayland
    } else if value.eq_ignore_ascii_case("x11") {
        DisplayServer::X11
    } else {
        DisplayServer::Unknown
    }
}

pub(super) fn ensure_cursor_support() -> Result<()> {
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    match from_session_type(session_type.as_deref()) {
        DisplayServer::Wayland => {
            bail!("cursor effects require X11; Wayland is not supported yet")
        }
        DisplayServer::X11 | DisplayServer::Unknown => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_type_detection_table() {
        let cases = [
            (Some("x11"), DisplayServer::X11),
            (Some("X11"), DisplayServer::X11),
            (Some("wayland"), DisplayServer::Wayland),
            (Some(" Wayland "), DisplayServer::Wayland),
            (Some("tty"), DisplayServer::Unknown),
            (Some(""), DisplayServer::Unknown),
            (None, DisplayServer::Unknown),
        ];
        for (input, expected) in cases {
            assert_eq!(from_session_type(input), expected, "input: {input:?}");
        }
    }
}
