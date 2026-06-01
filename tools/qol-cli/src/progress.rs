use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);
const PROGRESS_BAR_WIDTH: usize = 18;
const PROGRESS_PULSE_WIDTH: usize = 5;
const PROGRESS_CLEAR_WIDTH: usize = 80;
const CARGO_PROGRESS_FORMAT: &str = "json-render-diagnostics";

pub(crate) const COLOR_TITLE: &str = "1";
pub(crate) const COLOR_HINT: &str = "2";
pub(crate) const COLOR_TARGET: &str = "2";
pub(crate) const COLOR_PENDING: &str = "33";
pub(crate) const COLOR_SUCCESS: &str = "32";
pub(crate) const COLOR_INFO: &str = "36";
const STEP_LABEL_WIDTH: usize = 9;

#[derive(Clone, Copy)]
pub(crate) enum StepKind {
    Pending,
    Success,
    Info,
}

// ---------- public surface: titles, labels, runners ----------

pub(crate) fn print_title(text: &str) {
    println!("{}", paint_stdout(text, COLOR_TITLE));
}

pub(crate) fn print_hint(verbose: bool) {
    if verbose {
        return;
    }
    println!(
        "  {}",
        paint_stdout("hint: use -v/--verbose for detailed output", COLOR_HINT)
    );
}

pub(crate) fn step_label(verb: &str, kind: StepKind, target: &str) {
    let padded = format!("{verb:<STEP_LABEL_WIDTH$}");
    let painted_verb = paint_stdout(&padded, step_color(kind));
    let painted_target = paint_stdout(target, COLOR_TARGET);
    println!("  {painted_verb}{painted_target}");
}

pub(crate) fn run_step(
    verb: &str,
    kind: StepKind,
    target: &str,
    command: &mut Command,
    verbose: bool,
) -> Result<()> {
    if !verbose {
        step_label(verb, kind, target);
    }
    run_status(command, verbose)
}

pub(crate) fn run_step_inline(
    verb: &str,
    kind: StepKind,
    target: &str,
    command: &mut Command,
    verbose: bool,
) -> Result<()> {
    if verbose || !inline_clearing_enabled() {
        return run_step(verb, kind, target, command, verbose);
    }
    step_label(verb, kind, target);
    let cargo_progress = cargo_progress(command);
    let bar_visible = cargo_progress.is_some();
    let result = run_status_inner(command, cargo_progress);
    if result.is_ok() {
        clear_step_lines(bar_visible);
    }
    result
}

pub(crate) fn run_status(command: &mut Command, verbose: bool) -> Result<()> {
    if verbose {
        let status = command.status().context("failed to spawn command")?;
        if status.success() {
            return Ok(());
        }
        bail!("command failed with {status}");
    }
    let cargo_progress = cargo_progress(command);
    run_status_inner(command, cargo_progress)
}

pub(crate) fn run_silent(command: &mut Command) -> Result<()> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().context("failed to spawn command")?;
    let stdout = child
        .stdout
        .take()
        .map(|pipe| thread::spawn(move || read_pipe(pipe)));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(move || read_pipe(pipe)));
    let status = child.wait().context("failed waiting for command")?;
    let stdout = join_pipe(stdout)?;
    let stderr = join_pipe(stderr)?;
    if status.success() {
        return Ok(());
    }
    replay_bytes(&stdout);
    replay_bytes(&stderr);
    bail!("command failed with {status}");
}

// ---------- LoopProgress: outer bar across an iteration of steps ----------

pub(crate) struct LoopProgress {
    bar_label: &'static str,
    total: usize,
    done: usize,
    active: bool,
}

impl LoopProgress {
    pub(crate) fn new(bar_label: &'static str, total: usize, verbose: bool) -> Self {
        let active = !verbose && inline_clearing_enabled() && total > 1;
        if active {
            render_loop_bar(bar_label, 0, total);
            eprintln!();
        }
        Self {
            bar_label,
            total,
            done: 0,
            active,
        }
    }

    pub(crate) fn step_inline(
        &mut self,
        verb: &str,
        kind: StepKind,
        target: &str,
        command: &mut Command,
        verbose: bool,
    ) -> Result<()> {
        let result = if self.active {
            run_step_inline(verb, kind, target, command, verbose)
        } else {
            run_step(verb, kind, target, command, verbose)
        };
        self.tick(result.is_ok());
        result
    }

    pub(crate) fn step_silent(
        &mut self,
        verb: &str,
        kind: StepKind,
        target: &str,
        command: &mut Command,
        verbose: bool,
    ) -> Result<()> {
        if verbose {
            return run_status(command, true);
        }
        step_label(verb, kind, target);
        let result = run_silent(command);
        if result.is_ok() && self.active {
            eprint!("\x1b[A\x1b[2K\r");
            let _ = std::io::stderr().flush();
        }
        self.tick(result.is_ok());
        result
    }

    fn tick(&mut self, ok: bool) {
        if ok {
            self.done += 1;
        }
        if !self.active {
            return;
        }
        if ok {
            eprint!("\x1b[A\x1b[2K");
        }
        render_loop_bar(self.bar_label, self.done, self.total);
        eprintln!();
        let _ = std::io::stderr().flush();
    }

    pub(crate) fn finish(self) {
        if self.active {
            eprint!("\x1b[A\x1b[2K\r");
            let _ = std::io::stderr().flush();
        }
    }
}

// ---------- internals ----------

fn inline_clearing_enabled() -> bool {
    progress_enabled() && std::io::stdout().is_terminal()
}

fn clear_step_lines(bar_visible: bool) {
    let lines = if bar_visible { 2 } else { 1 };
    for _ in 0..lines {
        eprint!("\x1b[A\x1b[2K");
    }
    eprint!("\r");
    let _ = std::io::stderr().flush();
}

fn step_color(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Pending => COLOR_PENDING,
        StepKind::Success => COLOR_SUCCESS,
        StepKind::Info => COLOR_INFO,
    }
}

fn paint_stdout(text: &str, code: &str) -> String {
    if !color_enabled() || !std::io::stdout().is_terminal() {
        return text.to_string();
    }
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn paint_stderr(text: &str, code: &str) -> String {
    if !color_enabled() || !std::io::stderr().is_terminal() {
        return text.to_string();
    }
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn dim_stderr(text: &str) -> String {
    paint_stderr(text, "2")
}

fn color_enabled() -> bool {
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if env::var_os("TERM").is_some_and(|term| term == "dumb") {
        return false;
    }
    true
}

fn run_status_inner(command: &mut Command, cargo_progress: Option<CargoProgress>) -> Result<()> {
    if let Some(progress) = &cargo_progress {
        configure_cargo_progress(command, progress);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().context("failed to spawn command")?;
    let stdout = child.stdout.take().map(|pipe| {
        let cargo_progress = cargo_progress.clone();
        thread::spawn(move || read_stdout(pipe, cargo_progress))
    });
    let stderr = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(move || read_pipe(pipe)));
    let spinner = start_progress(&cargo_progress);
    let status = child.wait().context("failed waiting for command")?;
    drop(spinner);
    let stdout = join_pipe(stdout)?;
    let stderr = join_pipe(stderr)?;
    if status.success() {
        finish_progress(&cargo_progress);
        return Ok(());
    }
    clear_progress(&cargo_progress);
    replay_bytes(&stdout);
    replay_bytes(&stderr);
    bail!("command failed with {status}");
}

fn start_progress(cargo_progress: &Option<CargoProgress>) -> Option<ProgressSpinner> {
    if let Some(progress) = cargo_progress {
        render_cargo_progress(0, progress.total);
        return None;
    }
    Some(ProgressSpinner::start())
}

fn finish_progress(cargo_progress: &Option<CargoProgress>) {
    if let Some(progress) = cargo_progress {
        if render_cargo_progress(progress.total, progress.total) {
            eprintln!();
        }
    }
}

fn clear_progress(cargo_progress: &Option<CargoProgress>) {
    if cargo_progress.is_none() {
        return;
    }
    clear_progress_line();
}

fn read_stdout<R: Read>(
    pipe: R,
    cargo_progress: Option<CargoProgress>,
) -> std::io::Result<Vec<u8>> {
    match cargo_progress {
        Some(progress) => read_cargo_json(pipe, progress),
        None => read_pipe(pipe),
    }
}

fn read_pipe<R: Read>(mut pipe: R) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_cargo_json<R: Read>(pipe: R, progress: CargoProgress) -> std::io::Result<Vec<u8>> {
    let mut replay = Vec::new();
    let mut seen = HashSet::new();
    let reader = BufReader::new(pipe);
    for line in reader.lines() {
        let line = line?;
        read_cargo_json_line(&line, progress.total, &mut seen, &mut replay);
    }
    Ok(replay)
}

fn read_cargo_json_line(
    line: &str,
    total: usize,
    seen: &mut HashSet<String>,
    replay: &mut Vec<u8>,
) {
    if line.trim().is_empty() {
        return;
    }
    let parsed = match serde_json::from_str::<serde_json::Value>(line) {
        Ok(parsed) => parsed,
        Err(_) => {
            replay.extend_from_slice(line.as_bytes());
            replay.push(b'\n');
            return;
        }
    };
    if parsed.get("reason").and_then(serde_json::Value::as_str) == Some("compiler-artifact") {
        record_cargo_artifact(&parsed, total, seen);
        return;
    }
    if parsed.get("reason").and_then(serde_json::Value::as_str) == Some("compiler-message") {
        record_cargo_message(&parsed, replay);
    }
}

fn record_cargo_artifact(parsed: &serde_json::Value, total: usize, seen: &mut HashSet<String>) {
    let package_id = match parsed.get("package_id").and_then(serde_json::Value::as_str) {
        Some(package_id) => package_id,
        None => return,
    };
    if !seen.insert(package_id.to_string()) {
        return;
    }
    render_cargo_progress(seen.len().min(total), total);
}

fn record_cargo_message(parsed: &serde_json::Value, replay: &mut Vec<u8>) {
    let rendered = parsed
        .get("message")
        .and_then(|message| message.get("rendered"))
        .and_then(serde_json::Value::as_str);
    if let Some(rendered) = rendered {
        replay.extend_from_slice(rendered.as_bytes());
    }
}

fn join_pipe(handle: Option<JoinHandle<std::io::Result<Vec<u8>>>>) -> Result<Vec<u8>> {
    match handle {
        Some(handle) => handle
            .join()
            .map_err(|_| anyhow!("failed to read command output"))?
            .context("failed to read command output"),
        None => Ok(Vec::new()),
    }
}

fn replay_bytes(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    eprint!("{}", String::from_utf8_lossy(bytes));
}

// ---------- cargo progress: detection via cargo tree ----------

#[derive(Clone)]
struct CargoProgress {
    total: usize,
}

fn cargo_progress(command: &Command) -> Option<CargoProgress> {
    if !is_cargo_program(command.get_program()) {
        return None;
    }
    let args = command
        .get_args()
        .map(OsString::from)
        .collect::<Vec<OsString>>();
    let subcommand = args.first()?.to_str()?;
    if !is_cargo_compile_subcommand(subcommand) {
        return None;
    }
    let dependency_dir = cargo_dependency_dir(command, subcommand, &args)?;
    let total = cargo_dependency_total(
        &dependency_dir,
        &args,
        cargo_progress_includes_dev(subcommand),
    )
    .ok()?;
    if total == 0 {
        return None;
    }
    Some(CargoProgress { total })
}

fn is_cargo_program(program: &std::ffi::OsStr) -> bool {
    Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "cargo")
}

fn is_cargo_compile_subcommand(subcommand: &str) -> bool {
    matches!(subcommand, "build" | "check" | "install" | "test")
}

fn cargo_progress_includes_dev(subcommand: &str) -> bool {
    subcommand == "test"
}

fn cargo_dependency_dir(command: &Command, subcommand: &str, args: &[OsString]) -> Option<PathBuf> {
    let current_dir = command
        .get_current_dir()
        .map(Path::to_path_buf)
        .or_else(|| env::current_dir().ok())?;
    if subcommand == "install" {
        let path = cargo_arg_value(args, "--path")?;
        return Some(resolve_metadata_path(&current_dir, path));
    }
    Some(current_dir)
}

fn resolve_metadata_path(current_dir: &Path, path: OsString) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return path;
    }
    current_dir.join(path)
}

fn cargo_arg_value(args: &[OsString], name: &str) -> Option<OsString> {
    let inline_prefix = format!("{name}=");
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].to_string_lossy();
        if arg == name {
            return args.get(index + 1).cloned();
        }
        if let Some(value) = arg.strip_prefix(&inline_prefix) {
            return Some(OsString::from(value));
        }
        index += 1;
    }
    None
}

fn cargo_dependency_total(
    path: &Path,
    cargo_args: &[OsString],
    include_dev: bool,
) -> Result<usize> {
    let output = Command::new("cargo")
        .current_dir(path)
        .args(cargo_tree_args(cargo_args, include_dev))
        .stderr(Stdio::null())
        .output()
        .context("failed to run cargo tree")?;
    if !output.status.success() {
        bail!("cargo tree failed with {}", output.status);
    }
    cargo_tree_count(&output.stdout)
}

fn cargo_tree_args(cargo_args: &[OsString], include_dev: bool) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("tree"),
        OsString::from("--prefix"),
        OsString::from("none"),
        OsString::from("--edges"),
        cargo_tree_edges(include_dev),
    ];
    let mut index = 1;
    while index < cargo_args.len() {
        index = push_tree_arg(cargo_args, index, &mut args);
    }
    args
}

fn cargo_tree_edges(include_dev: bool) -> OsString {
    if include_dev {
        return OsString::from("normal,build,dev");
    }
    OsString::from("normal,build")
}

fn push_tree_arg(cargo_args: &[OsString], index: usize, args: &mut Vec<OsString>) -> usize {
    let arg = &cargo_args[index];
    let text = arg.to_string_lossy();
    if tree_value_arg(&text) {
        args.push(arg.clone());
        if let Some(value) = cargo_args.get(index + 1) {
            args.push(value.clone());
            return index + 2;
        }
        return index + 1;
    }
    if tree_inline_arg(&text) || tree_bool_arg(&text) {
        args.push(arg.clone());
    }
    index + 1
}

fn tree_value_arg(arg: &str) -> bool {
    matches!(arg, "--features" | "--manifest-path" | "--target")
}

fn tree_inline_arg(arg: &str) -> bool {
    arg.starts_with("--features=")
        || arg.starts_with("--manifest-path=")
        || arg.starts_with("--target=")
}

fn tree_bool_arg(arg: &str) -> bool {
    matches!(
        arg,
        "--all-features" | "--no-default-features" | "--locked" | "--offline" | "--frozen"
    )
}

fn cargo_tree_count(output: &[u8]) -> Result<usize> {
    let text = std::str::from_utf8(output).context("cargo tree output was not UTF-8")?;
    let packages = text
        .lines()
        .map(normalize_cargo_tree_line)
        .filter(|line| !line.is_empty())
        .collect::<HashSet<String>>();
    Ok(packages.len())
}

fn normalize_cargo_tree_line(line: &str) -> String {
    line.trim().trim_end_matches(" (*)").to_string()
}

fn configure_cargo_progress(command: &mut Command, _progress: &CargoProgress) {
    if !command_has_arg(command, "--quiet") && !command_has_arg(command, "-q") {
        command.arg("--quiet");
    }
    if command_has_arg(command, "--message-format")
        || command_has_arg_prefix(command, "--message-format=")
    {
        return;
    }
    command.arg("--message-format").arg(CARGO_PROGRESS_FORMAT);
}

fn command_has_arg(command: &Command, needle: &str) -> bool {
    command
        .get_args()
        .any(|arg| arg.to_str().is_some_and(|arg| arg == needle))
}

fn command_has_arg_prefix(command: &Command, prefix: &str) -> bool {
    command
        .get_args()
        .any(|arg| arg.to_str().is_some_and(|arg| arg.starts_with(prefix)))
}

// ---------- bars and spinner: ANSI rendering ----------

fn render_cargo_progress(done: usize, total: usize) -> bool {
    render_loop_bar("compile", done, total)
}

fn render_loop_bar(label: &str, done: usize, total: usize) -> bool {
    if !progress_enabled() {
        return false;
    }
    eprint!(
        "\r  {} {}",
        dim_stderr(label),
        cargo_progress_text(done, total)
    );
    let _ = std::io::stderr().flush();
    true
}

fn cargo_progress_text(done: usize, total: usize) -> String {
    let done = done.min(total);
    let percent = progress_percent(done, total);
    format!(
        "{done}/{total} {percent:>3}% {}",
        determinate_progress_bar(done, total)
    )
}

fn progress_percent(done: usize, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    ((done.min(total) * 100) / total).min(100)
}

fn determinate_progress_bar(done: usize, total: usize) -> String {
    let filled = determinate_progress_width(done, total);
    let mut bar = String::with_capacity(PROGRESS_BAR_WIDTH + 2);
    bar.push('[');
    for index in 0..PROGRESS_BAR_WIDTH {
        bar.push(determinate_progress_char(index, filled));
    }
    bar.push(']');
    bar
}

fn determinate_progress_width(done: usize, total: usize) -> usize {
    if total == 0 || done == 0 {
        return 0;
    }
    (done.min(total) * PROGRESS_BAR_WIDTH).div_ceil(total)
}

fn determinate_progress_char(index: usize, filled: usize) -> char {
    if index < filled {
        return '#';
    }
    '-'
}

fn clear_progress_line() {
    if !progress_enabled() {
        return;
    }
    eprint!("\r{}\r", " ".repeat(PROGRESS_CLEAR_WIDTH));
    let _ = std::io::stderr().flush();
}

struct ProgressSpinner {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ProgressSpinner {
    fn start() -> Self {
        let running = Arc::new(AtomicBool::new(true));
        if !progress_enabled() {
            return Self {
                running,
                handle: None,
            };
        }
        let spinner_running = Arc::clone(&running);
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut index = 0;
            while spinner_running.load(Ordering::Relaxed) {
                eprint!(
                    "\r  {} {} {}",
                    dim_stderr("progress"),
                    format_elapsed(started.elapsed()),
                    indeterminate_progress_bar(index)
                );
                let _ = std::io::stderr().flush();
                index += 1;
                thread::sleep(PROGRESS_INTERVAL);
            }
        });
        Self {
            running,
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
            clear_progress_line();
        }
    }
}

impl Drop for ProgressSpinner {
    fn drop(&mut self) {
        self.stop();
    }
}

fn progress_enabled() -> bool {
    if env::var_os("TERM").is_some_and(|term| term == "dumb") {
        return false;
    }
    std::io::stderr().is_terminal()
}

fn format_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn indeterminate_progress_bar(tick: usize) -> String {
    let head = tick % (PROGRESS_BAR_WIDTH + PROGRESS_PULSE_WIDTH);
    let mut bar = String::with_capacity(PROGRESS_BAR_WIDTH + 2);
    bar.push('[');
    for index in 0..PROGRESS_BAR_WIDTH {
        bar.push(indeterminate_progress_bar_char(head, index));
    }
    bar.push(']');
    bar
}

fn indeterminate_progress_bar_char(head: usize, index: usize) -> char {
    if head == index {
        return '>';
    }
    if head > index && head - index < PROGRESS_PULSE_WIDTH {
        return '=';
    }
    ' '
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_progress_elapsed_time() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "00:00");
        assert_eq!(format_elapsed(Duration::from_secs(65)), "01:05");
    }

    #[test]
    fn indeterminate_progress_bar_contains_visible_marker() {
        assert!(indeterminate_progress_bar(0).contains('>'));
        assert!(indeterminate_progress_bar(3).contains('='));
    }

    #[test]
    fn formats_cargo_progress_as_fraction_and_percent() {
        assert_eq!(cargo_progress_text(5, 10), "5/10  50% [#########---------]");
        assert_eq!(
            cargo_progress_text(10, 10),
            "10/10 100% [##################]"
        );
    }

    #[test]
    fn cargo_tree_args_keep_feature_flags() {
        let args = cargo_tree_args(
            &[
                "build".into(),
                "--release".into(),
                "--features".into(),
                "dev".into(),
                "--locked".into(),
            ],
            false,
        );
        assert_eq!(
            args,
            vec![
                OsString::from("tree"),
                OsString::from("--prefix"),
                OsString::from("none"),
                OsString::from("--edges"),
                OsString::from("normal,build"),
                OsString::from("--features"),
                OsString::from("dev"),
                OsString::from("--locked"),
            ]
        );
    }

    #[test]
    fn cargo_tree_args_include_dev_for_tests() {
        let args = cargo_tree_args(&["test".into()], true);
        assert_eq!(
            args,
            vec![
                OsString::from("tree"),
                OsString::from("--prefix"),
                OsString::from("none"),
                OsString::from("--edges"),
                OsString::from("normal,build,dev"),
            ]
        );
    }

    #[test]
    fn cargo_tree_count_deduplicates_packages() {
        let output = b"qol v0.1.0\nanyhow v1.0.0\nanyhow v1.0.0 (*)\n";
        assert_eq!(cargo_tree_count(output).unwrap(), 2);
    }

    #[test]
    fn cargo_tree_args_keep_target_flags() {
        let args = cargo_tree_args(
            &[
                "build".into(),
                "--target".into(),
                "x86_64-pc-windows-msvc".into(),
            ],
            false,
        );
        assert_eq!(
            args,
            vec![
                OsString::from("tree"),
                OsString::from("--prefix"),
                OsString::from("none"),
                OsString::from("--edges"),
                OsString::from("normal,build"),
                OsString::from("--target"),
                OsString::from("x86_64-pc-windows-msvc"),
            ]
        );
    }
}
