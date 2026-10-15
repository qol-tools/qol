use super::output;
use crate::updates::{Package, Unit, Update};

const GL_NVIDIA: &str = "org.freedesktop.Platform.GL.nvidia-";
const COLUMNS: &str = "--columns=ref,name,application,version,branch,origin,download-size";

#[derive(Debug, PartialEq)]
pub(super) struct Ref {
    pub(super) full: String,
    pub(super) app: bool,
    pub(super) id: String,
    pub(super) name: String,
    pub(super) branch: String,
    pub(super) bytes: Option<u64>,
}

pub(super) fn parse(text: &str) -> Vec<Ref> {
    text.lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.split('\t').collect();
            let full = *columns.first()?;
            let mut parts = full.split('/');
            let kind = parts.next()?;
            let id = parts.next()?.to_string();
            let name = columns.get(1).copied().unwrap_or_default();
            Some(Ref {
                full: full.to_string(),
                app: kind == "app",
                name: if name.is_empty() {
                    id.clone()
                } else {
                    name.to_string()
                },
                id,
                branch: columns.get(4).copied().unwrap_or_default().to_string(),
                bytes: columns.get(6).and_then(|size| parse_size(size)),
            })
        })
        .collect()
}

pub(super) fn parse_size(text: &str) -> Option<u64> {
    let text = text.replace('\u{a0}', " ");
    let (number, unit) = text.trim().split_once(' ')?;
    let number: f64 = number.replace(',', ".").parse().ok()?;
    let scale = match unit.trim() {
        "bytes" | "B" => 1.0,
        "kB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        _ => return None,
    };
    Some((number * scale).round() as u64)
}

pub(super) fn read() -> Result<Vec<Update>, String> {
    if !super::found("flatpak") {
        return Ok(Vec::new());
    }
    Ok(updates(parse(&output(
        "flatpak",
        &["remote-ls", "--updates", COLUMNS],
    )?)))
}

pub(super) fn updates(refs: Vec<Ref>) -> Vec<Update> {
    let (gl, others): (Vec<Ref>, Vec<Ref>) = refs
        .into_iter()
        .partition(|flatpak| flatpak.id.starts_with(GL_NVIDIA));
    let mut updates = Vec::new();
    if !gl.is_empty() {
        let count = gl.len();
        updates.push(update(
            "flatpak:gl-nvidia".into(),
            "NVIDIA GL runtimes".into(),
            format!("{count} Flatpak runtimes"),
            Unit::Runtimes,
            gl.into_iter()
                .map(|flatpak| {
                    let version = flatpak.id.trim_start_matches(GL_NVIDIA).replace('-', ".");
                    package(
                        &flatpak,
                        format!(
                            "NVIDIA {version} graphics for Flatpak apps · branch {}",
                            flatpak.branch
                        ),
                    )
                })
                .collect(),
        ));
    }
    for flatpak in others {
        let unit = if flatpak.app {
            Unit::Apps
        } else {
            Unit::Runtimes
        };
        let detail = format!(
            "{} · branch {}",
            if flatpak.app {
                "Flatpak app"
            } else {
                "Flatpak runtime"
            },
            flatpak.branch
        );
        updates.push(update(
            format!("flatpak:{}", flatpak.id),
            flatpak.name.clone(),
            flatpak.name.clone(),
            unit,
            vec![package(&flatpak, detail)],
        ));
    }
    updates
}

fn package(flatpak: &Ref, detail: String) -> Package {
    Package {
        name: flatpak.id.clone(),
        detail,
        bytes: flatpak.bytes,
        target: flatpak.full.clone(),
    }
}

fn update(id: String, label: String, name: String, unit: Unit, packages: Vec<Package>) -> Update {
    Update {
        id,
        tag: "Flatpak".into(),
        label,
        name,
        why: if unit == Unit::Apps {
            "A new version. Restarts nothing.".into()
        } else {
            "New builds for Flatpak apps. Restarts nothing.".into()
        },
        restart: false,
        held: None,
        unit,
        packages,
        running: None,
        finish: "Installed. Apps pick it up when reopened.".into(),
        stops_sound: false,
        caution: None,
        unattended: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "runtime/org.freedesktop.Platform.GL.nvidia-570-133-07/x86_64/1.4\tnvidia-570-133-07\torg.freedesktop.Platform.GL.nvidia-570-133-07\t\t1.4\tflathub\t299.0\u{a0}MB
runtime/org.freedesktop.Platform.GL.nvidia-580-173-02/x86_64/1.4\tnvidia-580-173-02\torg.freedesktop.Platform.GL.nvidia-580-173-02\t\t1.4\tflathub\t332.1\u{a0}MB
app/org.mozilla.firefox/x86_64/stable\tFirefox\torg.mozilla.firefox\t131.0\tstable\tflathub\t95.2\u{a0}MB
";

    #[test]
    fn refs_read_their_kind_name_and_size() {
        let refs = parse(LIST);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].id, "org.freedesktop.Platform.GL.nvidia-570-133-07");
        assert_eq!(refs[0].bytes, Some(299_000_000));
        assert!(!refs[0].app);
        assert!(refs[2].app);
        assert_eq!(refs[2].name, "Firefox");
        assert_eq!(parse_size("2.8\u{a0}kB"), Some(2_800));
        assert_eq!(parse_size("unknown"), None);
    }

    #[test]
    fn nvidia_gl_runtimes_are_one_update_and_apps_stand_alone() {
        let updates = updates(parse(LIST));
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].label, "NVIDIA GL runtimes");
        assert_eq!(updates[0].name, "2 Flatpak runtimes");
        assert_eq!(updates[0].wait(), "2 runtimes, restarts nothing");
        assert_eq!(
            updates[0].packages[1].detail,
            "NVIDIA 580.173.02 graphics for Flatpak apps · branch 1.4"
        );
        assert_eq!(
            updates[0].packages[0].target,
            "runtime/org.freedesktop.Platform.GL.nvidia-570-133-07/x86_64/1.4"
        );
        assert_eq!(updates[1].id, "flatpak:org.mozilla.firefox");
        assert_eq!(updates[1].wait(), "1 app, restarts nothing");
    }
}
