#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::ffi::OsString;
use std::fmt::Display;
use std::io::{self, Write};

use anyhow::{Context, Result};
use qol_headless::OutputFormat;
use serde_json::Value;

pub(crate) fn line(text: impl Display) {
    println!("{text}");
}

pub(crate) fn text(text: impl Display) {
    print!("{text}");
}

fn blank() {
    println!();
}

fn warn(text: impl Display) {
    let _ = writeln!(io::stderr(), "{text}");
}

fn lines(rows: Vec<String>) {
    for row in rows {
        line(row);
    }
}

pub(crate) fn capability(args: &[OsString]) -> Result<()> {
    line(crate::capability::run(args)?);
    Ok(())
}

pub(crate) fn spawn(args: &[OsString]) -> Result<()> {
    line(crate::spawn::run(args)?);
    Ok(())
}

pub(crate) fn fork(args: &[OsString]) -> Result<()> {
    line(crate::fork::run(args)?);
    Ok(())
}

pub(crate) fn forks(args: &[OsString]) -> Result<()> {
    lines(crate::fork::run_list(args)?);
    Ok(())
}

pub(crate) fn status(args: &[OsString]) -> Result<()> {
    lines(crate::status::run(args)?);
    Ok(())
}

pub(crate) fn switchboard(args: &[OsString]) -> Result<()> {
    lines(crate::switchboard::run(args, &mut |text| warn(text))?);
    Ok(())
}

pub(crate) fn close(args: &[OsString]) -> Result<()> {
    let outcome = crate::close::run(args)?;
    line(serde_json::to_string(&outcome).context("failed to serialize close outcome")?);
    Ok(())
}

pub(crate) fn loop_close(args: &[OsString], output_format: OutputFormat) -> Result<()> {
    let receipt = crate::close::run_loop_close(args)?;
    match output_format {
        OutputFormat::Json => line(
            serde_json::to_string_pretty(&receipt)
                .context("failed to serialize loop-close receipt")?,
        ),
        OutputFormat::PlainText => loop_close_summary(&receipt),
    }
    Ok(())
}

fn loop_close_summary(receipt: &Value) {
    line(format_args!(
        "loop_closed={} outcome={}",
        receipt["loop_closed"].as_bool().unwrap_or_default(),
        receipt["outcome"].as_str().unwrap_or_default()
    ));
    if let Some(state) = receipt.get("terminal_state").and_then(Value::as_str) {
        line(format_args!(
            "terminal_closed={} terminal_state={}",
            receipt["terminal_closed"].as_bool().unwrap_or_default(),
            state
        ));
    }
    if let Some(lanes) = receipt.get("sibling_lanes").and_then(Value::as_array) {
        line(format_args!("sibling lanes closed: {}", lanes.len()));
    }
    blank();
    line(receipt["final_report"].as_str().unwrap_or_default());
}

pub(crate) fn watch(args: &[OsString]) -> Result<()> {
    crate::watch::run(args, &mut io::stdout(), &mut io::stderr())
}

pub(crate) fn record(args: &[OsString]) -> Result<()> {
    let (stdout, code) = crate::record::execute(args);
    line(stdout);
    if code != 0 {
        std::process::exit(i32::from(code));
    }
    Ok(())
}

pub(crate) fn mcp(args: &[OsString]) -> Result<()> {
    if crate::mcp::is_help_request(args) {
        text(crate::mcp::help_text());
        return Ok(());
    }
    crate::mcp::run(args, io::stdout())
}

pub(crate) fn export(args: &[OsString]) -> Result<()> {
    text(crate::export::run(args)?);
    Ok(())
}
