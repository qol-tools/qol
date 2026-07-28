use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::ops::Deref;
use std::process::ExitCode;

pub const EXIT_SUCCESS: u8 = 0;
pub const EXIT_RUNTIME_ERROR: u8 = 1;
pub const EXIT_USAGE: u8 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    PlainText,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlainTextOutput {
    Empty,
    Text(String),
}

impl PlainTextOutput {
    pub fn empty() -> Self {
        Self::Empty
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    fn into_stdout(self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(text) => ensure_trailing_newline(text),
        }
    }
}

impl From<()> for PlainTextOutput {
    fn from(_: ()) -> Self {
        Self::Empty
    }
}

impl From<String> for PlainTextOutput {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for PlainTextOutput {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct CommandContext {
    command_path: Vec<String>,
    args: Vec<String>,
    output_format: OutputFormat,
}

impl CommandContext {
    pub fn command_path(&self) -> &[String] {
        &self.command_path
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn output_format(&self) -> OutputFormat {
        self.output_format
    }
}

type PlainTextHandler = Box<dyn Fn(&CommandContext) -> Result<PlainTextOutput> + Send + Sync>;
type ResultHandler = Box<dyn Fn(&CommandContext) -> Result<CommandResult> + Send + Sync>;
type StreamingHandler =
    Box<dyn Fn(&CommandContext, &mut dyn OutputSink) -> Result<u8> + Send + Sync>;
type JsonHandler = Box<dyn Fn(&CommandContext) -> Result<Value> + Send + Sync>;
type DoctorProvider = Box<dyn Fn() -> Result<Vec<DoctorCheckResult>> + Send + Sync>;
type DoctorAggregateProvider = Box<dyn Fn() -> Result<DoctorAggregateReport> + Send + Sync>;

enum ModeHandler {
    PlainText(PlainTextHandler),
    Result(ResultHandler),
    Streaming(StreamingHandler),
}

pub trait OutputSink {
    fn stdout(&mut self, text: &str);
    fn stderr(&mut self, text: &str);
}

#[derive(Debug, Default)]
struct BufferedOutputSink {
    stdout: String,
    stderr: String,
}

impl BufferedOutputSink {
    fn into_execution(self, exit_code: u8) -> Execution {
        Execution {
            stdout: self.stdout,
            stderr: self.stderr,
            exit_code,
        }
    }
}

impl OutputSink for BufferedOutputSink {
    fn stdout(&mut self, text: &str) {
        self.stdout.push_str(text);
    }

    fn stderr(&mut self, text: &str) {
        self.stderr.push_str(text);
    }
}

struct LiveOutputSink<'a, Stdout, Stderr>
where
    Stdout: Write,
    Stderr: Write,
{
    stdout: &'a mut Stdout,
    stderr: &'a mut Stderr,
}

impl<Stdout, Stderr> OutputSink for LiveOutputSink<'_, Stdout, Stderr>
where
    Stdout: Write,
    Stderr: Write,
{
    fn stdout(&mut self, text: &str) {
        write_and_flush_stream(&mut *self.stdout, text.as_bytes());
    }

    fn stderr(&mut self, text: &str) {
        write_and_flush_stream(&mut *self.stderr, text.as_bytes());
    }
}

pub struct Command {
    name: String,
    aliases: Vec<String>,
    about: String,
    usage: Option<String>,
    details: Vec<String>,
    output: Option<String>,
    exit_behavior: Option<String>,
    mode_handler: Option<ModeHandler>,
    json_handler: Option<JsonHandler>,
    subcommands: Vec<Command>,
}

impl Command {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            about: String::new(),
            usage: None,
            details: Vec::new(),
            output: None,
            exit_behavior: None,
            mode_handler: None,
            json_handler: None,
            subcommands: Vec::new(),
        }
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = about.into();
        self
    }

    pub fn usage(mut self, usage: impl Into<String>) -> Self {
        self.usage = Some(usage.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.details.push(detail.into());
        self
    }

    pub fn output(mut self, output: impl Into<String>) -> Self {
        self.output = Some(output.into());
        self
    }

    pub fn exit_behavior(mut self, exit_behavior: impl Into<String>) -> Self {
        self.exit_behavior = Some(exit_behavior.into());
        self
    }

    pub fn run_plain_text<F>(mut self, handler: F) -> Self
    where
        F: Fn(&CommandContext) -> Result<PlainTextOutput> + Send + Sync + 'static,
    {
        self.mode_handler = Some(ModeHandler::PlainText(Box::new(handler)));
        self
    }

    pub fn run_result<F>(mut self, handler: F) -> Self
    where
        F: Fn(&CommandContext) -> Result<CommandResult> + Send + Sync + 'static,
    {
        self.mode_handler = Some(ModeHandler::Result(Box::new(handler)));
        self
    }

    pub fn run_streaming<F>(mut self, handler: F) -> Self
    where
        F: Fn(&CommandContext, &mut dyn OutputSink) -> Result<u8> + Send + Sync + 'static,
    {
        self.mode_handler = Some(ModeHandler::Streaming(Box::new(handler)));
        self
    }

    pub fn run_json<F>(mut self, handler: F) -> Self
    where
        F: Fn(&CommandContext) -> Result<Value> + Send + Sync + 'static,
    {
        self.json_handler = Some(Box::new(handler));
        self
    }

    pub fn subcommand(mut self, command: Command) -> Self {
        self.subcommands.push(command);
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn supports_json(&self) -> bool {
        self.json_handler.is_some()
    }

    fn matches(&self, token: &str) -> bool {
        self.name == token || self.aliases.iter().any(|alias| alias == token)
    }

    fn find_subcommand(&self, token: &str) -> Option<&Command> {
        self.subcommands
            .iter()
            .find(|command| command.matches(token))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execution {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u8,
}

pub type CommandResult = Execution;

impl Execution {
    pub fn new(stdout: impl Into<String>, stderr: impl Into<String>, exit_code: u8) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code,
        }
    }

    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: EXIT_SUCCESS,
        }
    }

    pub fn usage(stderr: impl Into<String>) -> Self {
        Self {
            stdout: String::new(),
            stderr: ensure_trailing_newline(stderr.into()),
            exit_code: EXIT_USAGE,
        }
    }

    pub fn runtime_error(stderr: impl Into<String>) -> Self {
        Self {
            stdout: String::new(),
            stderr: ensure_trailing_newline(stderr.into()),
            exit_code: EXIT_RUNTIME_ERROR,
        }
    }

    pub fn as_exit_code(&self) -> ExitCode {
        ExitCode::from(self.exit_code)
    }

    pub fn emit(self) -> ExitCode {
        let mut stdout = io::stdout();
        let mut stderr = io::stderr();
        ExitCode::from(emit_execution(self, &mut stdout, &mut stderr))
    }
}

pub struct HeadlessApp {
    app_id: String,
    binary_name: String,
    about: String,
    default_command: Option<Vec<String>>,
    commands: Vec<Command>,
    doctor_checks: Vec<DoctorCheck>,
    doctor_provider: Option<DoctorProvider>,
    doctor_aggregate_provider: Option<DoctorAggregateProvider>,
}

impl HeadlessApp {
    pub fn new(app_id: impl Into<String>, binary_name: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            binary_name: binary_name.into(),
            about: String::new(),
            default_command: None,
            commands: Vec::new(),
            doctor_checks: Vec::new(),
            doctor_provider: None,
            doctor_aggregate_provider: None,
        }
    }

    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = about.into();
        self
    }

    pub fn default_command<I, S>(mut self, path: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.default_command = Some(path.into_iter().map(Into::into).collect());
        self
    }

    pub fn command(mut self, command: Command) -> Self {
        self.commands.push(command);
        self
    }

    pub fn doctor_check(mut self, check: DoctorCheck) -> Self {
        self.doctor_checks.push(check);
        self
    }

    pub fn doctor_checks<I>(mut self, checks: I) -> Self
    where
        I: IntoIterator<Item = DoctorCheck>,
    {
        self.doctor_checks.extend(checks);
        self
    }

    pub fn doctor_provider<F>(mut self, provider: F) -> Self
    where
        F: Fn() -> Result<Vec<DoctorCheckResult>> + Send + Sync + 'static,
    {
        self.doctor_provider = Some(Box::new(provider));
        self
    }

    pub fn doctor_aggregate_provider<F>(mut self, provider: F) -> Self
    where
        F: Fn() -> Result<DoctorAggregateReport> + Send + Sync + 'static,
    {
        self.doctor_aggregate_provider = Some(Box::new(provider));
        self
    }

    pub fn run(&self, args: impl IntoIterator<Item = String>) -> ExitCode {
        let mut stdout = io::stdout();
        let mut stderr = io::stderr();
        ExitCode::from(self.run_with_writers(args, &mut stdout, &mut stderr))
    }

    pub fn execute(&self, args: impl IntoIterator<Item = String>) -> Execution {
        match self.try_execute(args.into_iter().collect()) {
            Ok(execution) => execution,
            Err(error) => execution_from_dispatch_error(error),
        }
    }

    fn run_with_writers(
        &self,
        args: impl IntoIterator<Item = String>,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> u8 {
        let args = args.into_iter().collect::<Vec<_>>();
        match self.try_run_streaming(&args, stdout, stderr) {
            Ok(Some(exit_code)) => exit_code,
            Ok(None) => emit_execution(self.execute(args), stdout, stderr),
            Err(error) => emit_execution(execution_from_dispatch_error(error), stdout, stderr),
        }
    }

    fn try_execute(&self, args: Vec<String>) -> std::result::Result<Execution, DispatchError> {
        let (output_format, tokens) = split_output_format(args);
        if let Some(help_path) = extract_help_path(&tokens)? {
            if output_format == OutputFormat::Json {
                return Err(DispatchError::unsupported_json("help"));
            }
            return self.help_for(&help_path).map(Execution::success);
        }

        let tokens = match (tokens.is_empty(), &self.default_command) {
            (true, Some(default_command)) => default_command.clone(),
            (true, None) => {
                return Err(DispatchError::Usage(format!(
                    "No command supplied. Run `{}` help`.",
                    self.binary_name
                )))
            }
            (false, _) => tokens,
        };

        if tokens.first().map(String::as_str) == Some("doctor") {
            return self.execute_doctor(&tokens[1..], output_format);
        }

        let resolved = self.resolve_command_path(&tokens).ok_or_else(|| {
            DispatchError::Usage(format!(
                "Unknown command `{}`. Run `{}` help`.",
                tokens.join(" "),
                self.binary_name
            ))
        })?;

        let args = tokens[resolved.consumed..].to_vec();
        let context = CommandContext {
            command_path: resolved.path.clone(),
            args,
            output_format,
        };

        match output_format {
            OutputFormat::PlainText => match resolved.command.mode_handler.as_ref() {
                Some(ModeHandler::Result(handler)) => {
                    handler(&context).map_err(DispatchError::Runtime)
                }
                Some(ModeHandler::Streaming(handler)) => {
                    let mut sink = BufferedOutputSink::default();
                    let exit_code = handler(&context, &mut sink).map_err(DispatchError::Runtime)?;
                    Ok(sink.into_execution(exit_code))
                }
                Some(ModeHandler::PlainText(handler)) => {
                    let output = handler(&context).map_err(DispatchError::Runtime)?;
                    Ok(Execution::success(output.into_stdout()))
                }
                None => Err(DispatchError::Usage(format!(
                    "Command `{}` is not directly runnable.",
                    resolved.path.join(" ")
                ))),
            },
            OutputFormat::Json => {
                let handler = resolved
                    .command
                    .json_handler
                    .as_ref()
                    .ok_or_else(|| DispatchError::unsupported_json(&resolved.path.join(" ")))?;
                let value = handler(&context).map_err(DispatchError::Runtime)?;
                let stdout = json_stdout(&value).map_err(DispatchError::Runtime)?;
                Ok(Execution::success(stdout))
            }
        }
    }

    fn try_run_streaming(
        &self,
        args: &[String],
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> std::result::Result<Option<u8>, DispatchError> {
        let (output_format, tokens) = split_output_format(args.to_vec());
        if extract_help_path(&tokens)?.is_some() {
            return Ok(None);
        }

        let tokens = match (tokens.is_empty(), &self.default_command) {
            (true, Some(default_command)) => default_command.clone(),
            (true, None) => {
                return Err(DispatchError::Usage(format!(
                    "No command supplied. Run `{}` help`.",
                    self.binary_name
                )))
            }
            (false, _) => tokens,
        };

        if tokens.first().map(String::as_str) == Some("doctor") {
            return Ok(None);
        }

        let resolved = self.resolve_command_path(&tokens).ok_or_else(|| {
            DispatchError::Usage(format!(
                "Unknown command `{}`. Run `{}` help`.",
                tokens.join(" "),
                self.binary_name
            ))
        })?;

        if output_format == OutputFormat::Json {
            return Ok(None);
        }

        let Some(ModeHandler::Streaming(handler)) = resolved.command.mode_handler.as_ref() else {
            return Ok(None);
        };

        let context = CommandContext {
            command_path: resolved.path.clone(),
            args: tokens[resolved.consumed..].to_vec(),
            output_format,
        };
        let mut sink = LiveOutputSink { stdout, stderr };
        let exit_code = handler(&context, &mut sink).map_err(DispatchError::Runtime)?;
        Ok(Some(exit_code))
    }

    fn resolve_command_path(&self, tokens: &[String]) -> Option<ResolvedCommand<'_>> {
        let first = tokens.first()?;
        let mut command = self
            .commands
            .iter()
            .find(|command| command.matches(first))?;
        let mut path = vec![command.name.clone()];
        let mut consumed = 1;

        for token in tokens.iter().skip(1) {
            let Some(subcommand) = command.find_subcommand(token) else {
                break;
            };
            command = subcommand;
            path.push(command.name.clone());
            consumed += 1;
        }

        Some(ResolvedCommand {
            command,
            path,
            consumed,
        })
    }

    fn help_for(&self, path: &[String]) -> std::result::Result<String, DispatchError> {
        if path.is_empty() {
            return Ok(self.general_help());
        }

        if path.first().map(String::as_str) == Some("doctor") {
            return self.doctor_help(&path[1..]);
        }

        let resolved = self.resolve_command_path(path).ok_or_else(|| {
            DispatchError::Usage(format!(
                "Unknown help topic `{}`. Run `{}` help`.",
                path.join(" "),
                self.binary_name
            ))
        })?;

        if resolved.consumed != path.len() {
            return Err(DispatchError::Usage(format!(
                "Unknown help topic `{}`. Run `{}` help`.",
                path.join(" "),
                self.binary_name
            )));
        }

        Ok(self.command_help(resolved.command, &resolved.path))
    }

    fn general_help(&self) -> String {
        let mut lines = vec![self.binary_name.clone()];
        if !self.about.is_empty() {
            lines.extend(["".to_string(), self.about.clone()]);
        }

        lines.extend([
            "".to_string(),
            "Usage:".to_string(),
            format!("  {} <command> [args]", self.binary_name),
            format!("  {} help <command>", self.binary_name),
            format!("  {} <command> help", self.binary_name),
        ]);

        if let Some(default_command) = &self.default_command {
            lines.push(format!(
                "  {}  # {}",
                self.binary_name,
                default_command.join(" ")
            ));
        }

        let mut command_rows = self
            .commands
            .iter()
            .map(|command| (command.name.as_str(), command.about.as_str()))
            .collect::<Vec<_>>();
        if self.has_doctor() {
            command_rows.push(("doctor", "Run read-only health checks."));
        }
        command_rows.sort_by(|left, right| left.0.cmp(right.0));

        if !command_rows.is_empty() {
            lines.extend(["".to_string(), "Commands:".to_string()]);
            for (name, about) in command_rows {
                lines.push(format_command_row(name, about));
            }
        }

        lines.extend([
            "".to_string(),
            "Global flags:".to_string(),
            "  --json  Request structured JSON output from commands that support it.".to_string(),
        ]);

        ensure_trailing_newline(lines.join("\n"))
    }

    fn command_help(&self, command: &Command, path: &[String]) -> String {
        let command_path = path.join(" ");
        let mut lines = vec![format!("{} {}", self.binary_name, command_path)];

        if !command.about.is_empty() {
            lines.extend(["".to_string(), command.about.clone()]);
        }

        lines.extend([
            "".to_string(),
            "Usage:".to_string(),
            format!(
                "  {}",
                command
                    .usage
                    .clone()
                    .unwrap_or_else(|| format!("{} {}", self.binary_name, command_path))
            ),
            format!("  {} {} help", self.binary_name, command_path),
            format!("  {} help {}", self.binary_name, command_path),
        ]);

        if !command.details.is_empty() {
            lines.extend(["".to_string(), "Details:".to_string()]);
            lines.extend(command.details.iter().map(|detail| format!("  {detail}")));
        }

        lines.extend(["".to_string(), "Output:".to_string()]);
        if let Some(output) = &command.output {
            lines.push(format!("  {output}"));
        } else {
            lines.push("  Plain-text stdout; diagnostics on stderr.".to_string());
        }
        lines.push(if command.supports_json() {
            "  Supports --json.".to_string()
        } else {
            "  Does not support --json.".to_string()
        });

        if let Some(exit_behavior) = &command.exit_behavior {
            lines.extend([
                "".to_string(),
                "Exit:".to_string(),
                format!("  {exit_behavior}"),
            ]);
        }

        if !command.subcommands.is_empty() {
            lines.extend(["".to_string(), "Subcommands:".to_string()]);
            for subcommand in &command.subcommands {
                lines.push(format_command_row(&subcommand.name, &subcommand.about));
            }
        }

        ensure_trailing_newline(lines.join("\n"))
    }

    fn doctor_help(&self, path: &[String]) -> std::result::Result<String, DispatchError> {
        if !self.has_doctor() {
            return Err(DispatchError::Usage(format!(
                "`{}` does not register doctor checks.",
                self.binary_name
            )));
        }

        if path.is_empty() {
            return Ok(self.doctor_command_help());
        }

        if path.len() != 1 {
            return Err(DispatchError::Usage(format!(
                "Unknown doctor help topic `{}`.",
                path.join(" ")
            )));
        }

        let check = self
            .doctor_checks
            .iter()
            .find(|check| check.id == path[0])
            .ok_or_else(|| DispatchError::Usage(format!("Unknown doctor check `{}`.", path[0])))?;

        let lines = [
            format!("{} doctor {}", self.binary_name, check.id),
            String::new(),
            check.about.clone(),
            String::new(),
            "Usage:".to_string(),
            format!("  {} doctor {}", self.binary_name, check.id),
            format!("  {} --json doctor {}", self.binary_name, check.id),
            format!("  {} doctor {} help", self.binary_name, check.id),
            String::new(),
            "Output:".to_string(),
            "  Plain-text check result by default.".to_string(),
            "  Supports --json and returns the standard doctor report shape.".to_string(),
            String::new(),
            "Exit:".to_string(),
            "  Exits 0 when the check runs; inspect the report status for ok, warn, or fail."
                .to_string(),
        ];

        Ok(ensure_trailing_newline(lines.join("\n")))
    }

    fn doctor_command_help(&self) -> String {
        let mut lines = vec![
            format!("{} doctor", self.binary_name),
            String::new(),
            "Run read-only health checks.".to_string(),
            String::new(),
            "Usage:".to_string(),
            format!("  {} doctor", self.binary_name),
            format!("  {} --json doctor", self.binary_name),
            format!("  {} doctor --json", self.binary_name),
        ];
        if !self.doctor_checks.is_empty() {
            lines.push(format!("  {} doctor <check-id>", self.binary_name));
        }
        lines.extend([
            format!("  {} doctor help", self.binary_name),
            String::new(),
            "Checks:".to_string(),
        ]);

        for check in &self.doctor_checks {
            lines.push(format_command_row(&check.id, &check.about));
        }
        if self.doctor_provider.is_some() {
            lines.push("  Additional checks are discovered when doctor runs.".to_string());
        }
        if self.doctor_aggregate_provider.is_some() {
            lines.push("  Host and plugin checks are discovered when doctor runs.".to_string());
        }

        lines.extend([
            String::new(),
            "Output:".to_string(),
            "  Plain-text report by default.".to_string(),
        ]);
        if self.doctor_aggregate_provider.is_some() {
            lines.push("  Supports --json and returns status, host, and plugins.".to_string());
        } else {
            lines.push("  Supports --json and returns plugin_id, status, and checks.".to_string());
        }
        lines.extend([String::new(), "Exit:".to_string()]);
        if self.doctor_aggregate_provider.is_some() {
            lines.push("  Exits 0 when healthy, 1 for warnings, and 2 for failures.".to_string());
        } else {
            lines.push(
                "  Exits 0 when checks run; inspect the report status for ok, warn, or fail."
                    .to_string(),
            );
        }

        ensure_trailing_newline(lines.join("\n"))
    }

    fn execute_doctor(
        &self,
        args: &[String],
        output_format: OutputFormat,
    ) -> std::result::Result<Execution, DispatchError> {
        if !self.has_doctor() {
            return Err(DispatchError::Usage(format!(
                "`{}` does not register doctor checks.",
                self.binary_name
            )));
        }

        if args.len() > 1 {
            return Err(DispatchError::Usage(format!(
                "Unknown doctor command `{}`. Run `{} doctor help`.",
                args.join(" "),
                self.binary_name
            )));
        }

        let selected = args.first().map(String::as_str);
        if selected.is_none() {
            if let Some(provider) = &self.doctor_aggregate_provider {
                let report = provider().map_err(DispatchError::Runtime)?;
                return self.execute_doctor_aggregate(&report, output_format);
            }
        }
        let checks = self.selected_doctor_checks(selected)?;
        let mut results = checks
            .into_iter()
            .map(DoctorCheck::run_check)
            .collect::<Vec<_>>();
        if selected.is_none() {
            if let Some(provider) = &self.doctor_provider {
                results.extend(provider().map_err(DispatchError::Runtime)?);
            }
        }
        let report = DoctorReport::from_results(self.app_id.clone(), results);

        match output_format {
            OutputFormat::PlainText => {
                Ok(Execution::success(self.doctor_plain_text_output(&report)))
            }
            OutputFormat::Json => {
                let stdout = json_stdout(&report).map_err(DispatchError::Runtime)?;
                Ok(Execution::success(stdout))
            }
        }
    }

    fn execute_doctor_aggregate(
        &self,
        report: &DoctorAggregateReport,
        output_format: OutputFormat,
    ) -> std::result::Result<Execution, DispatchError> {
        let stdout = match output_format {
            OutputFormat::PlainText => self.doctor_aggregate_plain_text_output(report),
            OutputFormat::Json => json_stdout(report).map_err(DispatchError::Runtime)?,
        };
        Ok(Execution::new(
            stdout,
            String::new(),
            aggregate_doctor_exit_code(report.status),
        ))
    }

    fn selected_doctor_checks(
        &self,
        selected: Option<&str>,
    ) -> std::result::Result<Vec<&DoctorCheck>, DispatchError> {
        let Some(selected) = selected else {
            return Ok(self.doctor_checks.iter().collect());
        };

        self.doctor_checks
            .iter()
            .find(|check| check.id == selected)
            .map(|check| vec![check])
            .ok_or_else(|| DispatchError::Usage(format!("Unknown doctor check `{selected}`.")))
    }

    fn has_doctor(&self) -> bool {
        !self.doctor_checks.is_empty()
            || self.doctor_provider.is_some()
            || self.doctor_aggregate_provider.is_some()
    }

    fn doctor_plain_text_output(&self, report: &DoctorReport) -> String {
        let mut lines = vec![format!(
            "{} doctor: {}",
            report.plugin_id,
            report.status.as_str()
        )];

        for check in &report.checks {
            lines.push(format!(
                "[{}] {} - {}",
                check.status.as_str(),
                check.id,
                check.message
            ));
            if let Some(fix) = &check.fix {
                lines.push(format!("  fix: {fix}"));
            }
        }

        ensure_trailing_newline(lines.join("\n"))
    }

    fn doctor_aggregate_plain_text_output(&self, report: &DoctorAggregateReport) -> String {
        let mut lines = vec![
            format!("{} doctor: {}", self.app_id, report.status.as_str()),
            String::new(),
            format!(
                "Host {}: {}",
                report.host.plugin_id,
                report.host.status.as_str()
            ),
        ];
        push_doctor_results(&mut lines, &report.host.checks, "  ");

        for plugin in &report.plugins {
            lines.extend([
                String::new(),
                format!("Plugin {}: {}", plugin.plugin_id, plugin.status.as_str()),
            ]);
            if !plugin.diagnostics.is_empty() {
                lines.push("  Diagnostics:".to_string());
                push_doctor_results(&mut lines, &plugin.diagnostics, "    ");
            }
            if let Some(plugin_report) = &plugin.report {
                lines.push(format!(
                    "  Report {}: {}",
                    plugin_report.plugin_id,
                    plugin_report.status.as_str()
                ));
                push_doctor_results(&mut lines, &plugin_report.checks, "    ");
            }
        }

        ensure_trailing_newline(lines.join("\n"))
    }
}

struct ResolvedCommand<'a> {
    command: &'a Command,
    path: Vec<String>,
    consumed: usize,
}

#[derive(Debug)]
enum DispatchError {
    Usage(String),
    Runtime(anyhow::Error),
}

impl DispatchError {
    fn unsupported_json(command_path: &str) -> Self {
        Self::Usage(format!("Command `{command_path}` does not support --json."))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorStatus {
    Ok,
    Warn,
    Fail,
}

impl DoctorStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    fn aggregate(results: &[DoctorCheckResult]) -> Self {
        Self::aggregate_statuses(results.iter().map(|result| result.status))
    }

    fn aggregate_statuses(statuses: impl IntoIterator<Item = DoctorStatus>) -> Self {
        let mut aggregate = Self::Ok;
        for status in statuses {
            if status == Self::Fail {
                return Self::Fail;
            }
            if status == Self::Warn {
                aggregate = Self::Warn;
            }
        }
        aggregate
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PluginDoctorReport {
    pub plugin_id: String,
    pub status: DoctorStatus,
    pub diagnostics: Vec<DoctorCheckResult>,
    pub report: Option<PreservedDoctorReport>,
}

impl PluginDoctorReport {
    pub fn new(
        plugin_id: impl Into<String>,
        diagnostics: Vec<DoctorCheckResult>,
        report: Option<DoctorReport>,
    ) -> Self {
        Self::new_preserved(
            plugin_id,
            diagnostics,
            report.map(PreservedDoctorReport::new),
        )
    }

    pub fn new_preserved(
        plugin_id: impl Into<String>,
        diagnostics: Vec<DoctorCheckResult>,
        report: Option<PreservedDoctorReport>,
    ) -> Self {
        let status = DoctorStatus::aggregate_statuses(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.status)
                .chain(report.iter().map(|report| report.status)),
        );
        Self {
            plugin_id: plugin_id.into(),
            status,
            diagnostics,
            report,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreservedDoctorReport {
    report: DoctorReport,
    raw: Value,
}

impl PreservedDoctorReport {
    pub fn new(report: DoctorReport) -> Self {
        let raw = serde_json::to_value(&report)
            .expect("serializing a doctor report containing JSON values cannot fail");
        Self { report, raw }
    }

    pub fn from_value(raw: Value) -> serde_json::Result<Self> {
        let report = serde_json::from_value(raw.clone())?;
        Ok(Self { report, raw })
    }

    pub fn from_slice(bytes: &[u8]) -> serde_json::Result<Self> {
        Self::from_value(serde_json::from_slice(bytes)?)
    }

    pub fn raw(&self) -> &Value {
        &self.raw
    }
}

impl Deref for PreservedDoctorReport {
    type Target = DoctorReport;

    fn deref(&self) -> &Self::Target {
        &self.report
    }
}

impl Serialize for PreservedDoctorReport {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PreservedDoctorReport {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = Value::deserialize(deserializer)?;
        Self::from_value(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DoctorAggregateReport {
    pub status: DoctorStatus,
    pub host: DoctorReport,
    pub plugins: Vec<PluginDoctorReport>,
}

impl DoctorAggregateReport {
    pub fn new(host: DoctorReport, mut plugins: Vec<PluginDoctorReport>) -> Self {
        plugins.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
        let status = DoctorStatus::aggregate_statuses(
            std::iter::once(host.status).chain(plugins.iter().map(|plugin| plugin.status)),
        );
        Self {
            status,
            host,
            plugins,
        }
    }
}

fn push_doctor_results(lines: &mut Vec<String>, results: &[DoctorCheckResult], indent: &str) {
    for result in results {
        lines.push(format!(
            "{indent}[{}] {} - {}",
            result.status.as_str(),
            result.id,
            result.message
        ));
        if let Some(fix) = &result.fix {
            lines.push(format!("{indent}  fix: {fix}"));
        }
    }
}

pub struct DoctorCheck {
    id: String,
    about: String,
    handler: Box<dyn Fn() -> Result<DoctorCheckResult> + Send + Sync>,
}

impl DoctorCheck {
    pub fn new<F>(id: impl Into<String>, about: impl Into<String>, handler: F) -> Self
    where
        F: Fn() -> Result<DoctorCheckResult> + Send + Sync + 'static,
    {
        Self {
            id: id.into(),
            about: about.into(),
            handler: Box::new(handler),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn about(&self) -> &str {
        &self.about
    }

    fn run_check(&self) -> DoctorCheckResult {
        match (self.handler)() {
            Ok(mut result) => {
                if result.id.is_empty() {
                    result.id = self.id.clone();
                }
                result
            }
            Err(error) => DoctorCheckResult::fail(
                self.id.clone(),
                format!("{} failed: {error:#}", self.about),
            ),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DoctorReport {
    pub plugin_id: String,
    pub status: DoctorStatus,
    pub checks: Vec<DoctorCheckResult>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl DoctorReport {
    pub fn from_results(plugin_id: impl Into<String>, checks: Vec<DoctorCheckResult>) -> Self {
        let status = DoctorStatus::aggregate(&checks);
        Self {
            plugin_id: plugin_id.into(),
            status,
            checks,
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DoctorCheckResult {
    pub id: String,
    pub status: DoctorStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl DoctorCheckResult {
    pub fn ok(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, DoctorStatus::Ok, message)
    }

    pub fn warn(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, DoctorStatus::Warn, message)
    }

    pub fn fail(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, DoctorStatus::Fail, message)
    }

    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    fn new(id: impl Into<String>, status: DoctorStatus, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status,
            message: message.into(),
            fix: None,
            details: None,
            extensions: BTreeMap::new(),
        }
    }
}

fn aggregate_doctor_exit_code(status: DoctorStatus) -> u8 {
    match status {
        DoctorStatus::Ok => 0,
        DoctorStatus::Warn => 1,
        DoctorStatus::Fail => 2,
    }
}

fn split_output_format(args: Vec<String>) -> (OutputFormat, Vec<String>) {
    let mut output_format = OutputFormat::PlainText;
    let mut tokens = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--json" => output_format = OutputFormat::Json,
            "--help" => tokens.push("help".to_string()),
            _ => tokens.push(arg),
        }
    }

    (output_format, tokens)
}

fn extract_help_path(tokens: &[String]) -> std::result::Result<Option<Vec<String>>, DispatchError> {
    let help_positions = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| (token == "help").then_some(index))
        .collect::<Vec<_>>();

    match help_positions.as_slice() {
        [] => Ok(None),
        [0] => Ok(Some(tokens[1..].to_vec())),
        [position] if *position == tokens.len() - 1 => Ok(Some(tokens[..*position].to_vec())),
        [_] => Err(DispatchError::Usage(
            "`help` must be the first token or final token.".to_string(),
        )),
        _ => Err(DispatchError::Usage(
            "`help` may appear only once in a command.".to_string(),
        )),
    }
}

fn json_stdout(value: &impl Serialize) -> Result<String> {
    let mut stdout = serde_json::to_string(value)?;
    stdout.push('\n');
    Ok(stdout)
}

fn format_command_row(name: &str, about: &str) -> String {
    if about.is_empty() {
        return format!("  {name}");
    }
    format!("  {name:<18} {about}")
}

fn ensure_trailing_newline(mut text: String) -> String {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn execution_from_dispatch_error(error: DispatchError) -> Execution {
    match error {
        DispatchError::Usage(message) => Execution::usage(message),
        DispatchError::Runtime(error) => Execution::runtime_error(format!("{error:#}")),
    }
}

fn emit_execution(execution: Execution, stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    let exit_code = execution.exit_code;
    write_stream(stdout, execution.stdout.as_bytes());
    write_stream(stderr, execution.stderr.as_bytes());
    exit_code
}

fn write_stream(stream: &mut impl Write, bytes: &[u8]) {
    let _ = stream.write_all(bytes);
}

fn write_and_flush_stream(stream: &mut impl Write, bytes: &[u8]) {
    write_stream(stream, bytes);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn app() -> HeadlessApp {
        HeadlessApp::new("plugin-test", "test-bin")
            .about("Test app.")
            .default_command(["toggle"])
            .command(
                Command::new("toggle")
                    .about("Toggle something.")
                    .run_plain_text(|_| Ok(PlainTextOutput::text("toggled"))),
            )
            .command(
                Command::new("status")
                    .about("Show status.")
                    .run_plain_text(|_| Ok(PlainTextOutput::text("running")))
                    .run_json(|_| Ok(json!({ "status": "running" }))),
            )
            .command(
                Command::new("bounded")
                    .about("Return a command result.")
                    .run_result(|_| {
                        Ok(CommandResult::new(
                            "bounded stdout\n",
                            "bounded stderr\n",
                            7,
                        ))
                    }),
            )
            .command(
                Command::new("stream")
                    .about("Write through an output sink.")
                    .run_streaming(|_, sink| {
                        sink.stdout("stream stdout 1\n");
                        sink.stderr("stream stderr\n");
                        sink.stdout("stream stdout 2\n");
                        Ok(9)
                    }),
            )
            .doctor_check(DoctorCheck::new(
                "required_binaries",
                "Check binaries.",
                || {
                    Ok(DoctorCheckResult::warn(
                        "required_binaries",
                        "ffmpeg is missing",
                    ))
                },
            ))
    }

    #[test]
    fn no_args_run_default_command() {
        let execution = app().execute(Vec::new());
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "toggled\n");
    }

    #[test]
    fn help_first_and_final_are_equivalent() {
        let first = app().execute(vec!["help".to_string(), "status".to_string()]);
        let final_token = app().execute(vec!["status".to_string(), "help".to_string()]);
        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Supports --json"));
    }

    #[test]
    fn help_in_middle_is_rejected() {
        let execution = app().execute(vec![
            "status".to_string(),
            "help".to_string(),
            "extra".to_string(),
        ]);
        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("first token or final token"));
    }

    #[test]
    fn json_before_and_after_doctor_are_equivalent() {
        let before = app().execute(vec!["--json".to_string(), "doctor".to_string()]);
        let after = app().execute(vec!["doctor".to_string(), "--json".to_string()]);
        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);

        let value: Value = serde_json::from_str(&before.stdout).unwrap();
        assert_eq!(value["plugin_id"], "plugin-test");
        assert_eq!(value["status"], "warn");
        assert_eq!(value["checks"][0]["id"], "required_binaries");
    }

    #[test]
    fn doctor_report_json_round_trips_through_the_shared_contract() {
        let value = json!({
            "plugin_id": "plugin-test",
            "status": "warn",
            "checks": [
                {
                    "id": "required_binaries",
                    "status": "warn",
                    "message": "ffmpeg is missing",
                    "fix": "Install ffmpeg.",
                    "details": {
                        "binary": "ffmpeg"
                    }
                },
                {
                    "id": "runtime_dirs",
                    "status": "ok",
                    "message": "Runtime directories are ready"
                }
            ]
        });

        let report: DoctorReport = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(report.plugin_id, "plugin-test");
        assert_eq!(report.status, DoctorStatus::Warn);
        assert_eq!(serde_json::to_value(report).unwrap(), value);
    }

    #[test]
    fn doctor_aggregate_json_round_trips_without_flattening_plugin_reports() {
        let value = json!({
            "status": "fail",
            "host": {
                "plugin_id": "host-test",
                "status": "warn",
                "checks": [
                    {
                        "id": "host-second",
                        "status": "warn",
                        "message": "second host check"
                    },
                    {
                        "id": "host-first",
                        "status": "ok",
                        "message": "first host check"
                    }
                ]
            },
            "plugins": [
                {
                    "plugin_id": "plugin-z",
                    "status": "fail",
                    "diagnostics": [
                        {
                            "id": "invocation",
                            "status": "fail",
                            "message": "plugin invocation failed"
                        }
                    ],
                    "report": null
                },
                {
                    "plugin_id": "plugin-a",
                    "status": "warn",
                    "diagnostics": [],
                    "report": {
                        "plugin_id": "plugin-a",
                        "status": "warn",
                        "schema_version": 2,
                        "checks": [
                            {
                                "id": "second",
                                "status": "warn",
                                "message": "second plugin check",
                                "fix": null,
                                "future_metric": {
                                    "latency_ms": 7
                                }
                            },
                            {
                                "id": "first",
                                "status": "ok",
                                "message": "first plugin check"
                            }
                        ]
                    }
                }
            ]
        });

        let report: DoctorAggregateReport = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(report.status, DoctorStatus::Fail);
        assert_eq!(
            report.plugins[1].report.as_ref().unwrap().checks[0].id,
            "second"
        );
        assert_eq!(
            report.plugins[1].report.as_ref().unwrap().raw()["schema_version"],
            2
        );
        assert_eq!(serde_json::to_value(report).unwrap(), value);
    }

    #[test]
    fn aggregate_constructors_derive_status_and_sort_only_plugin_groups() {
        let host = DoctorReport::from_results(
            "host-test",
            vec![
                DoctorCheckResult::warn("host-second", "second host check"),
                DoctorCheckResult::ok("host-first", "first host check"),
            ],
        );
        let plugin_report = DoctorReport::from_results(
            "plugin-z",
            vec![
                DoctorCheckResult::warn("second", "second plugin check"),
                DoctorCheckResult::ok("first", "first plugin check"),
            ],
        );
        let plugin_z = PluginDoctorReport::new(
            "plugin-z",
            vec![
                DoctorCheckResult::ok("diagnostic-second", "second diagnostic"),
                DoctorCheckResult::fail("diagnostic-first", "first diagnostic"),
            ],
            Some(plugin_report),
        );
        let plugin_a = PluginDoctorReport::new(
            "plugin-a",
            vec![DoctorCheckResult::ok("ready", "plugin is ready")],
            None,
        );

        let aggregate = DoctorAggregateReport::new(host, vec![plugin_z, plugin_a]);

        assert_eq!(aggregate.status, DoctorStatus::Fail);
        assert_eq!(
            aggregate
                .plugins
                .iter()
                .map(|plugin| plugin.plugin_id.as_str())
                .collect::<Vec<_>>(),
            vec!["plugin-a", "plugin-z"]
        );
        assert_eq!(aggregate.host.checks[0].id, "host-second");
        assert_eq!(aggregate.plugins[1].status, DoctorStatus::Fail);
        assert_eq!(aggregate.plugins[1].diagnostics[0].id, "diagnostic-second");
        assert_eq!(
            aggregate.plugins[1].report.as_ref().unwrap().checks[0].id,
            "second"
        );
    }

    #[test]
    fn aggregate_doctor_exit_codes_preserve_health_gate_semantics() {
        assert_eq!(aggregate_doctor_exit_code(DoctorStatus::Ok), 0);
        assert_eq!(aggregate_doctor_exit_code(DoctorStatus::Warn), 1);
        assert_eq!(aggregate_doctor_exit_code(DoctorStatus::Fail), 2);
    }

    #[test]
    fn doctor_provider_is_lazy_and_runs_only_after_doctor_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider_calls = Arc::clone(&calls);
        let app = HeadlessApp::new("host-test", "host-test")
            .command(
                Command::new("status")
                    .about("Show status.")
                    .run_plain_text(|_| Ok(PlainTextOutput::text("running"))),
            )
            .doctor_provider(move || {
                provider_calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![DoctorCheckResult::warn(
                    "plugin-test/required_binaries",
                    "ffmpeg is missing",
                )])
            });

        let help = app.execute(vec!["help".to_string(), "doctor".to_string()]);
        let status = app.execute(vec!["status".to_string()]);

        assert_eq!(help.exit_code, EXIT_SUCCESS);
        assert!(help.stdout.contains("discovered when doctor runs"));
        assert!(!help.stdout.contains("doctor <check-id>"));
        assert_eq!(status.stdout, "running\n");
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let doctor = app.execute(vec!["doctor".to_string(), "--json".to_string()]);
        let report: DoctorReport = serde_json::from_str(&doctor.stdout).unwrap();

        assert_eq!(doctor.exit_code, EXIT_SUCCESS);
        assert_eq!(report.status, DoctorStatus::Warn);
        assert_eq!(report.checks[0].id, "plugin-test/required_binaries");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn doctor_aggregate_provider_is_lazy_and_renders_grouped_plain_text() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider_calls = Arc::clone(&calls);
        let app = HeadlessApp::new("host-test", "host-test")
            .command(
                Command::new("status")
                    .about("Show status.")
                    .run_plain_text(|_| Ok(PlainTextOutput::text("running"))),
            )
            .doctor_aggregate_provider(move || {
                provider_calls.fetch_add(1, Ordering::SeqCst);
                let host = DoctorReport::from_results(
                    "host-test",
                    vec![DoctorCheckResult::warn("runtime", "host needs attention")
                        .with_fix("Repair host.")],
                );
                let plugin_z = PluginDoctorReport::new(
                    "plugin-z",
                    vec![DoctorCheckResult::fail("invocation", "plugin failed")
                        .with_fix("Reinstall plugin.")],
                    None,
                );
                let plugin_a = PluginDoctorReport::new(
                    "plugin-a",
                    Vec::new(),
                    Some(DoctorReport::from_results(
                        "plugin-a",
                        vec![DoctorCheckResult::ok("ready", "plugin ready")],
                    )),
                );
                Ok(DoctorAggregateReport::new(host, vec![plugin_z, plugin_a]))
            });

        let help = app.execute(vec!["help".to_string(), "doctor".to_string()]);
        let status = app.execute(vec!["status".to_string()]);

        assert_eq!(help.exit_code, EXIT_SUCCESS);
        assert!(help.stdout.contains("returns status, host, and plugins"));
        assert!(!help.stdout.contains("doctor <check-id>"));
        assert_eq!(status.stdout, "running\n");
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let doctor = app.execute(vec!["doctor".to_string()]);

        assert_eq!(doctor.exit_code, 2);
        assert_eq!(
            doctor.stdout,
            concat!(
                "host-test doctor: fail\n",
                "\n",
                "Host host-test: warn\n",
                "  [warn] runtime - host needs attention\n",
                "    fix: Repair host.\n",
                "\n",
                "Plugin plugin-a: ok\n",
                "  Report plugin-a: ok\n",
                "    [ok] ready - plugin ready\n",
                "\n",
                "Plugin plugin-z: fail\n",
                "  Diagnostics:\n",
                "    [fail] invocation - plugin failed\n",
                "      fix: Reinstall plugin.\n",
            )
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn doctor_provider_failure_is_a_runtime_error_with_clean_stdout() {
        let app = HeadlessApp::new("host-test", "host-test")
            .doctor_provider(|| Err(anyhow::anyhow!("plugin aggregation failed")));

        let execution = app.execute(vec!["--json".to_string(), "doctor".to_string()]);

        assert_eq!(execution.exit_code, EXIT_RUNTIME_ERROR);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("plugin aggregation failed"));
    }

    #[test]
    fn unsupported_json_is_rejected_before_handler_runs() {
        let execution = app().execute(vec!["--json".to_string(), "toggle".to_string()]);
        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("does not support --json"));
    }

    #[test]
    fn command_json_handler_runs_when_registered() {
        let execution = app().execute(vec!["status".to_string(), "--json".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "{\"status\":\"running\"}\n");
    }

    #[test]
    fn command_result_handler_can_return_stdout_stderr_and_nonzero_exit() {
        let execution = app().execute(vec!["bounded".to_string()]);
        assert_eq!(execution.exit_code, 7);
        assert_eq!(execution.stdout, "bounded stdout\n");
        assert_eq!(execution.stderr, "bounded stderr\n");
    }

    #[test]
    fn streaming_handler_writes_to_sink_and_returns_exit_code() {
        let execution = app().execute(vec!["stream".to_string()]);
        assert_eq!(execution.exit_code, 9);
        assert_eq!(execution.stdout, "stream stdout 1\nstream stdout 2\n");
        assert_eq!(execution.stderr, "stream stderr\n");
    }

    #[test]
    fn run_streaming_handler_writes_to_supplied_streams() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_code =
            app().run_with_writers(vec!["stream".to_string()], &mut stdout, &mut stderr);

        assert_eq!(exit_code, 9);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "stream stdout 1\nstream stdout 2\n"
        );
        assert_eq!(String::from_utf8(stderr).unwrap(), "stream stderr\n");
    }

    #[test]
    fn streaming_handler_json_is_rejected_by_shared_gate() {
        let execution = app().execute(vec!["stream".to_string(), "--json".to_string()]);
        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("does not support --json"));
    }

    #[test]
    fn contextual_doctor_check_help_is_supported() {
        let first = app().execute(vec![
            "help".to_string(),
            "doctor".to_string(),
            "required_binaries".to_string(),
        ]);
        let final_token = app().execute(vec![
            "doctor".to_string(),
            "required_binaries".to_string(),
            "help".to_string(),
        ]);
        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("test-bin doctor required_binaries"));
    }
}
