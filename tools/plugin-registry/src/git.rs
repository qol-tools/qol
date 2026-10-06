use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct PluginAtRevision {
    pub commit: String,
    pub dir: String,
    pub plugin_toml: String,
}

pub fn plugin_at(revision: &str, plugin_id: &str) -> Result<PluginAtRevision> {
    let commit = text(&git(
        &["rev-parse", &format!("{revision}^{{commit}}")],
        None,
    )?)?
    .trim()
    .to_string();
    let dirs = text(&git(
        &["ls-tree", "--name-only", "-d", &format!("{commit}:plugins")],
        None,
    )?)?;
    let dirs: Vec<&str> = dirs.lines().collect();
    let requests: String = dirs
        .iter()
        .map(|dir| format!("{commit}:plugins/{dir}/plugin.toml\n"))
        .collect();
    let manifests = cat_file_batch(&git(&["cat-file", "--batch"], Some(requests.as_bytes()))?)?;
    let mut matches = dirs.iter().zip(manifests).filter_map(|(dir, manifest)| {
        let manifest = manifest?;
        (crate::release::declared_id(&manifest).as_deref() == Some(plugin_id))
            .then(|| (dir.to_string(), manifest))
    });
    let (dir, plugin_toml) = matches
        .next()
        .with_context(|| format!("{revision}: no plugin directory declares {plugin_id}"))?;
    if matches.next().is_some() {
        anyhow::bail!("{revision}: more than one plugin directory declares {plugin_id}");
    }
    Ok(PluginAtRevision {
        commit,
        dir,
        plugin_toml,
    })
}

pub fn tree_tar(commit: &str, dir: &str) -> Result<Vec<u8>> {
    git(
        &[
            "archive",
            "--format=tar",
            &format!("{commit}:plugins/{dir}"),
        ],
        None,
    )
}

fn git(args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not run git {}", args.join(" ")))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn text(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).context("git printed text that is not UTF-8")
}

fn cat_file_batch(mut output: &[u8]) -> Result<Vec<Option<String>>> {
    let mut objects = Vec::new();
    while !output.is_empty() {
        let end = output
            .iter()
            .position(|b| *b == b'\n')
            .context("git cat-file printed an unterminated header")?;
        let header = std::str::from_utf8(&output[..end])?;
        output = &output[end + 1..];
        if header.ends_with(" missing") {
            objects.push(None);
            continue;
        }
        let size: usize = header
            .rsplit(' ')
            .next()
            .and_then(|size| size.parse().ok())
            .with_context(|| format!("git cat-file printed an unreadable header {header:?}"))?;
        let body = output
            .get(..size)
            .context("git cat-file printed a short object")?;
        objects.push(Some(text(body)?));
        output = output.get(size + 1..).unwrap_or_default();
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cat_file_batch_reads_objects_and_missing_entries_in_order() {
        let output = b"aaa blob 5\nhello\nHEAD:x missing\nbbb blob 0\n\n";
        let objects = cat_file_batch(output).unwrap();
        assert_eq!(
            objects,
            vec![Some("hello".to_string()), None, Some(String::new())]
        );
    }
}
