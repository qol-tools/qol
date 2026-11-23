use std::ffi::OsString;
use std::fmt;
use std::path::Path;

use serde_json::{json, Value};

pub(crate) const EXIT_FAILED: u8 = 1;
pub(crate) const EXIT_USAGE: u8 = 2;
pub(crate) const EXIT_UNKNOWN: u8 = 3;
pub(crate) const EXIT_REFUSED: u8 = 4;

pub(crate) struct Outcome {
    pub(crate) stdout: String,
    pub(crate) code: u8,
}

impl Outcome {
    pub(crate) fn ok(value: Value) -> Self {
        Self {
            stdout: value.to_string(),
            code: 0,
        }
    }

    pub(crate) fn failed(code: u8, message: impl fmt::Display) -> Self {
        Self {
            stdout: json!({ "error": message.to_string() }).to_string(),
            code,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Unknown(String),
    Refused(String),
    Failed(anyhow::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(message) | Self::Refused(message) => formatter.write_str(message),
            Self::Failed(error) => write!(formatter, "{error:#}"),
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(error.into())
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) enum Failure {
    Usage(String),
    Store(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Store(error)
    }
}

pub(crate) fn execute(
    args: &[OsString],
    noun: &str,
    help: &str,
    run: impl FnOnce(&str, &Flags) -> std::result::Result<Outcome, Failure>,
) -> Outcome {
    let usage = |message: String| Outcome::failed(EXIT_USAGE, format!("{message}\n\n{help}"));
    let Some((verb, rest)) = args.split_first() else {
        return usage(format!("a {noun} verb is required"));
    };
    let Some(verb) = verb.to_str() else {
        return usage(format!("the {noun} verb must be valid UTF-8"));
    };
    if matches!(verb, "help" | "-h" | "--help") {
        return Outcome {
            stdout: help.to_owned(),
            code: 0,
        };
    }
    let flags = match Flags::parse(rest) {
        Ok(flags) => flags,
        Err(message) => return usage(message),
    };
    match run(verb, &flags) {
        Ok(outcome) => outcome,
        Err(Failure::Usage(message)) => usage(message),
        Err(Failure::Store(Error::Unknown(message))) => Outcome::failed(EXIT_UNKNOWN, message),
        Err(Failure::Store(Error::Refused(message))) => Outcome::failed(EXIT_REFUSED, message),
        Err(Failure::Store(error)) => Outcome::failed(EXIT_FAILED, error),
    }
}

pub(crate) struct Flags {
    values: Vec<(String, String)>,
}

impl Flags {
    fn parse(args: &[OsString]) -> std::result::Result<Self, String> {
        let mut values: Vec<(String, String)> = Vec::new();
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let name = flag
                .to_str()
                .and_then(|flag| flag.strip_prefix("--"))
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("unexpected argument `{}`", flag.to_string_lossy()))?;
            let value = args
                .next()
                .ok_or_else(|| format!("--{name} needs a value"))?
                .to_str()
                .ok_or_else(|| format!("--{name} must be valid UTF-8"))?;
            if values.iter().any(|(seen, _)| seen == name) {
                return Err(format!("--{name} was given twice"));
            }
            values.push((name.to_owned(), value.to_owned()));
        }
        Ok(Self { values })
    }

    pub(crate) fn only(&self, allowed: &[&str]) -> std::result::Result<(), Failure> {
        match self
            .values
            .iter()
            .find(|(name, _)| !allowed.contains(&name.as_str()))
        {
            Some((name, _)) => Err(Failure::Usage(format!("unknown flag --{name}"))),
            None => Ok(()),
        }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(flag, _)| flag == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn required(&self, name: &str) -> std::result::Result<&str, Failure> {
        self.get(name)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| Failure::Usage(format!("--{name} is required")))
    }

    pub(crate) fn absolute(&self, name: &str) -> std::result::Result<&str, Failure> {
        let value = self.required(name)?;
        if Path::new(value).is_absolute() {
            Ok(value)
        } else {
            Err(Failure::Usage(format!("--{name} must be an absolute path")))
        }
    }
}
