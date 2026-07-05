use std::{env, ffi::OsStr, fs, path::PathBuf, process};

fn main() {
    match run() {
        Ok(()) => {}
        Err(CliError::Usage(message)) => {
            eprintln!("{message}");
            process::exit(2);
        }
        Err(CliError::Runtime(message)) => {
            eprintln!("{message}");
            process::exit(1);
        }
    }
}

fn run() -> Result<(), CliError> {
    let mut args = env::args_os().skip(1).collect::<Vec<_>>();
    let profile = parse_profile(&mut args)?;
    match args.as_slice() {
        [] => {
            print!("{}", render(profile));
            Ok(())
        }
        [flag, path] if flag == OsStr::new("--check") => check(profile, PathBuf::from(path)),
        [flag, path] if flag == OsStr::new("--write") => write(profile, PathBuf::from(path)),
        _ => Err(CliError::Usage(usage())),
    }
}

fn parse_profile(args: &mut Vec<std::ffi::OsString>) -> Result<Profile, CliError> {
    let Some(first) = args.first() else {
        return Ok(Profile::Core);
    };
    if first != OsStr::new("--profile") {
        return Ok(Profile::Core);
    }
    if args.len() < 2 {
        return Err(CliError::Usage(usage()));
    }
    let value = args.remove(1);
    args.remove(0);
    match value.to_str() {
        Some("core") => Ok(Profile::Core),
        Some("plugin-keyremap") => Ok(Profile::PluginKeyremap),
        Some("plugin-lights") => Ok(Profile::PluginLights),
        Some("alt-tab-cinnamon") => Ok(Profile::AltTabCinnamon),
        _ => Err(CliError::Usage(usage())),
    }
}

fn check(profile: Profile, path: PathBuf) -> Result<(), CliError> {
    let expected = render(profile);
    let actual = fs::read_to_string(&path).map_err(|err| {
        CliError::Runtime(format!(
            "qol-theme-css: failed to read {}: {err}",
            path.display()
        ))
    })?;
    if actual == expected {
        return Ok(());
    }
    Err(CliError::Runtime(format!(
        "qol-theme-css: {} is stale; run `cargo run -q -p qol-theme --bin qol-theme-css -- {}--write {}`",
        path.display(),
        profile.flag_hint(),
        path.display()
    )))
}

fn write(profile: Profile, path: PathBuf) -> Result<(), CliError> {
    fs::write(&path, render(profile)).map_err(|err| {
        CliError::Runtime(format!(
            "qol-theme-css: failed to write {}: {err}",
            path.display()
        ))
    })
}

fn usage() -> String {
    "usage: qol-theme-css [--profile core|plugin-keyremap|plugin-lights|alt-tab-cinnamon] [--check <path> | --write <path>]"
        .to_string()
}

#[derive(Clone, Copy)]
enum Profile {
    Core,
    PluginKeyremap,
    PluginLights,
    AltTabCinnamon,
}

impl Profile {
    fn flag_hint(self) -> &'static str {
        match self {
            Self::Core => "",
            Self::PluginKeyremap => "--profile plugin-keyremap ",
            Self::PluginLights => "--profile plugin-lights ",
            Self::AltTabCinnamon => "--profile alt-tab-cinnamon ",
        }
    }
}

fn render(profile: Profile) -> String {
    match profile {
        Profile::Core => qol_theme::css::dark_css(),
        Profile::PluginKeyremap => qol_theme::css::plugin_keyremap_css(),
        Profile::PluginLights => qol_theme::css::plugin_lights_css(),
        Profile::AltTabCinnamon => qol_theme::css::alt_tab_cinnamon_js(),
    }
}

enum CliError {
    Usage(String),
    Runtime(String),
}
