use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::path::Path;

pub fn unpack_zip(archive: &Path, dir: &Path) -> Result<()> {
    let file = File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("{} is not a zip archive", archive.display()))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .with_context(|| format!("reading entry {index} of {}", archive.display()))?;
        let Some(relative) = entry.enclosed_name() else {
            bail!(
                "{} holds `{}`, which points outside the build",
                archive.display(),
                entry.name()
            );
        };
        if entry.is_symlink() {
            bail!("{} holds the link `{}`", archive.display(), entry.name());
        }
        let target = dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("creating {}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut out =
            File::create(&target).with_context(|| format!("writing {}", target.display()))?;
        std::io::copy(&mut entry, &mut out)
            .with_context(|| format!("unpacking {}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn archive(root: &Path, entries: &[(&str, &[u8])]) -> std::path::PathBuf {
        let path = root.join("build.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    #[test]
    fn a_build_unpacks_into_its_folder() {
        let root = tempfile::tempdir().unwrap();
        let zip = archive(
            root.path(),
            &[
                ("manifest.json", b"{}"),
                ("plugins/qol-demo-v1.0.0/qol-demo-linux-x86_64", b"plugin"),
                ("tray/qol-tray", b"tray"),
            ],
        );
        let dir = root.path().join("out");
        unpack_zip(&zip, &dir).unwrap();
        assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), b"{}");
        assert_eq!(
            fs::read(dir.join("plugins/qol-demo-v1.0.0/qol-demo-linux-x86_64")).unwrap(),
            b"plugin"
        );
        assert_eq!(fs::read(dir.join("tray/qol-tray")).unwrap(), b"tray");
    }

    #[test]
    fn an_entry_outside_the_folder_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let zip = archive(root.path(), &[("../escape", b"x")]);
        let dir = root.path().join("out");
        let error = format!("{:#}", unpack_zip(&zip, &dir).unwrap_err());
        assert!(error.contains("outside the build"), "{error}");
        assert!(!root.path().join("escape").exists());
    }

    #[test]
    fn a_file_that_is_not_a_zip_says_so() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("build.zip");
        fs::write(&path, b"not a zip").unwrap();
        let error = format!("{:#}", unpack_zip(&path, root.path()).unwrap_err());
        assert!(error.contains("not a zip archive"), "{error}");
    }
}
