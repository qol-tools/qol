use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Topic {
    All,
    Focus,
    Monitor,
    Boot,
    Opacity,
    Ui,
    Preview,
    Shot,
}

impl Topic {
    pub(super) fn parse(value: &str) -> Result<Self> {
        match value {
            "all" => Ok(Self::All),
            "focus" => Ok(Self::Focus),
            "monitor" => Ok(Self::Monitor),
            "boot" => Ok(Self::Boot),
            "opacity" => Ok(Self::Opacity),
            "ui" => Ok(Self::Ui),
            "preview" => Ok(Self::Preview),
            "shot" => Ok(Self::Shot),
            _ => bail!("unknown trace topic `{value}`"),
        }
    }

    pub(super) fn matches(self, tag: &str) -> bool {
        match self {
            Self::All => true,
            Self::Shot => tag.starts_with("SHOT_"),
            Self::Ui => tag.starts_with("LAUNCHER_") || tag.starts_with("WORLD_"),
            Self::Preview => {
                tag.starts_with("PREVIEW_")
                    || tag.starts_with("REFRESH_")
                    || tag.starts_with("CAPTURE")
                    || matches!(
                        tag,
                        "SHOW_RECV" | "SHOW_TIMING" | "SHOW_PAINTED" | "FOCUS_WIN"
                    )
            }
            Self::Focus => matches!(
                tag,
                "FOCUS"
                    | "FOCUS_WIN"
                    | "ACTIVATE"
                    | "ACTIVATE_WIN"
                    | "WM_RECEIVE"
                    | "ALT_POLL_START"
                    | "DISMISS"
            ),
            Self::Monitor => {
                matches!(
                    tag,
                    "PUBLISH"
                        | "SUBSCRIBE"
                        | "RECV"
                        | "LEGEND"
                        | "AMC"
                        | "HOST_EMIT_AMC"
                        | "PLUGIN_RECV_AMC"
                )
            }
            Self::Boot => matches!(tag, "PUBLISH" | "SUBSCRIBE" | "RECV" | "LEGEND"),
            Self::Opacity => matches!(
                tag,
                "SHOW_WIN" | "HIDE_WIN" | "GHOSTWIN" | "GHOSTDUMP" | "SUMMARY"
            ),
        }
    }
}

pub(super) struct Args {
    pub(super) plugin: Option<String>,
    pub(super) topic: Topic,
    pub(super) grep: Option<String>,
    pub(super) since: Option<Duration>,
    pub(super) mark: Option<String>,
    pub(super) replay: bool,
    pub(super) details: bool,
    pub(super) stats: bool,
    pub(super) anomalies: bool,
    pub(super) no_ghosts: bool,
    pub(super) no_opacity: bool,
    pub(super) no_header: bool,
}

impl Args {
    #[cfg(test)]
    pub(super) fn parse(args: &[OsString]) -> Result<Self> {
        Self::parse_for("trace-rs", args)
    }

    pub(super) fn parse_for(command_name: &str, args: &[OsString]) -> Result<Self> {
        let mut plugin = None;
        let mut topic = Topic::All;
        let mut grep = None;
        let mut since = None;
        let mut mark = None;
        let mut replay = false;
        let mut details = false;
        let mut stats = false;
        let mut anomalies = false;
        let mut no_ghosts = false;
        let mut no_opacity = false;
        let mut no_header = false;
        let mut focus_only = false;

        let mut iter = args.iter().peekable();
        while let Some(arg) = iter.next() {
            let value = arg
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("{command_name} argument is not valid UTF-8"))?;
            match value {
                "-h" | "--help" => {
                    print_help(command_name);
                    std::process::exit(0);
                }
                "-f" | "--focus-only" => focus_only = true,
                "-g" | "--no-ghosts" => no_ghosts = true,
                "-o" | "--no-opacity" => no_opacity = true,
                "--no-header" => no_header = true,
                "-d" | "--details" => details = true,
                "--replay" => replay = true,
                "--anomalies" => anomalies = true,
                "--stats" => stats = true,
                "--topic" => topic = Topic::parse(next_value(&mut iter, "--topic")?)?,
                "--grep" => grep = Some(next_value(&mut iter, "--grep")?.to_string()),
                "--since" => since = Some(parse_duration(next_value(&mut iter, "--since")?)?),
                "--mark" => mark = Some(next_value(&mut iter, "--mark")?.to_string()),
                _ if value.starts_with("--topic=") => {
                    topic = Topic::parse(value.trim_start_matches("--topic="))?
                }
                _ if value.starts_with("--grep=") => {
                    grep = Some(value.trim_start_matches("--grep=").to_string())
                }
                _ if value.starts_with("--since=") => {
                    since = Some(parse_duration(value.trim_start_matches("--since="))?)
                }
                _ if value.starts_with("--mark=") => {
                    mark = Some(value.trim_start_matches("--mark=").to_string())
                }
                "focus" => focus_only = true,
                positional if positional.starts_with('-') => {
                    bail!("unknown {command_name} flag `{positional}`")
                }
                positional => {
                    if plugin.is_some() {
                        bail!("usage: qol {command_name} [plugin|focus] [flags]");
                    }
                    plugin = Some(positional.to_string());
                }
            }
        }

        if focus_only {
            topic = Topic::Focus;
        }
        if plugin.as_deref() == Some("runtime") {
            plugin = None;
        }

        Ok(Self {
            plugin,
            topic,
            grep,
            since,
            mark,
            replay,
            details,
            stats,
            anomalies,
            no_ghosts,
            no_opacity,
            no_header,
        })
    }
}

pub(super) fn next_value<'a>(
    iter: &mut std::iter::Peekable<std::slice::Iter<'a, OsString>>,
    flag: &str,
) -> Result<&'a str> {
    iter.next()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
}

pub(super) fn parse_duration(value: &str) -> Result<Duration> {
    let digits = value
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        bail!("invalid duration `{value}`");
    }
    let amount = digits.parse::<u64>()?;
    let suffix = &value[digits.len()..];
    let seconds = match suffix {
        "" | "s" => amount,
        "m" => amount * 60,
        "h" => amount * 60 * 60,
        _ => bail!("invalid duration unit `{suffix}`"),
    };
    Ok(Duration::from_secs(seconds))
}

pub(super) fn print_help(command_name: &str) {
    println!(
        "qol {command_name} [plugin|focus] [flags]\n\
         \n\
         Runtime trace formatter for {DEFAULT_LOG_FILE}.\n\
         \n\
         Flags:\n\
           -f, --focus-only        focus events only\n\
           -g, --no-ghosts         hide GHOSTDUMP/GHOSTWIN/SUMMARY rows\n\
           -o, --no-opacity        hide HIDE_WIN/SHOW_WIN rows\n\
           -d, --details           start with expanded detail lines\n\
               --topic <name>      all, focus, monitor, boot, opacity, ui, preview, shot\n\
               --grep <text>       filter output by substring\n\
               --since <duration>  filter events since duration, e.g. 5s, 10m, 1h\n\
               --mark <text>       append a marker to the raw trace log and exit\n\
               --stats             print focus/opacity summaries on exit\n\
               --replay            process the existing log from the start and exit\n\
               --anomalies         show only anomalies"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_legacy_focus() {
        let args = Args::parse(&["focus".into(), "--details".into()]).unwrap();
        assert_eq!(args.topic, Topic::Focus);
        assert!(args.details);
    }

    #[test]
    fn parse_args_keeps_stats_flag() {
        let args = Args::parse(&["--stats".into(), "--replay".into()]).unwrap();
        assert!(args.stats);
        assert!(args.replay);
    }

    #[test]
    fn no_header_flag_suppresses_the_banner() {
        assert!(Args::parse(&["--no-header".into()]).unwrap().no_header);
        assert!(!Args::parse(&[]).unwrap().no_header);
    }

    #[test]
    fn duration_parser_handles_units() {
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }
}
