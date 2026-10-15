use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn get(schema: &str, key: &str) -> Result<String> {
    Ok(unquote(run(&["get", schema, key])?.trim()))
}

pub fn list(schema: &str) -> Result<Vec<(String, String)>> {
    Ok(parse_list(schema, &run(&["list-recursively", schema])?))
}

pub fn set(schema: &str, key: &str, value: &str) -> Result<()> {
    let status = Command::new("gsettings")
        .args(["set", schema, key, value])
        .status()
        .context("failed to run gsettings")?;
    if !status.success() {
        bail!("gsettings set {schema} {key} {value} failed");
    }
    Ok(())
}

fn run(args: &[&str]) -> Result<String> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .context("failed to run gsettings")?;
    if !output.status.success() {
        bail!(
            "gsettings {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_list(schema: &str, text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            if parts.next()? != schema {
                return None;
            }
            let key = parts.next()?;
            let value = parts.next()?;
            Some((key.to_string(), unquote(value.trim())))
        })
        .collect()
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .unwrap_or(value)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_keeps_only_the_asked_schema_and_unquotes_like_get() {
        let text = "org.cinnamon.desktop.peripherals.mouse accel-profile 'flat'\n\
                    org.cinnamon.desktop.peripherals.mouse speed 0.25\n\
                    org.cinnamon.desktop.peripherals.mouse.child key true\n\
                    org.cinnamon.desktop.peripherals.mouse double-click 400\n\
                    org.cinnamon.desktop.peripherals.keyboard delay uint32 500\n";
        assert_eq!(
            parse_list("org.cinnamon.desktop.peripherals.mouse", text),
            [
                ("accel-profile".to_string(), "flat".to_string()),
                ("speed".to_string(), "0.25".to_string()),
                ("double-click".to_string(), "400".to_string()),
            ]
        );
        assert_eq!(
            parse_list("org.cinnamon.desktop.peripherals.keyboard", text),
            [("delay".to_string(), "uint32 500".to_string())]
        );
    }

    #[test]
    fn a_value_with_spaces_stays_whole() {
        let text = "org.gnome.desktop.input-sources sources [('xkb', 'us'), ('xkb', 'dk')]\n";
        assert_eq!(
            parse_list("org.gnome.desktop.input-sources", text),
            [(
                "sources".to_string(),
                "[('xkb', 'us'), ('xkb', 'dk')]".to_string()
            )]
        );
    }

    #[test]
    fn only_a_fully_quoted_value_loses_its_quotes() {
        assert_eq!(unquote("'Mint-Y'"), "Mint-Y");
        assert_eq!(unquote("'half"), "'half");
        assert_eq!(unquote("true"), "true");
    }
}
