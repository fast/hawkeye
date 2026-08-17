// Copyright 2026 FastLabs Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fmt;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ArgAction;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use exn::ResultExt;
use hawkeye::Engine;
use hawkeye::Mode;
use hawkeye::Plan;
use hawkeye::Report;
use hawkeye::Status;
use logforth::filter::rustlog::RustLogFilterBuilder;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Command {
    #[arg(long, global = true, help = "path to the config file")]
    config: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        value_enum,
        help = "output format",
        default_value_t = OutputFormat::Human
    )]
    output_format: OutputFormat,

    #[command(subcommand)]
    subcommand: SubcommandOptions,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Subcommand)]
enum SubcommandOptions {
    /// Check whether selected files have canonical headers.
    Check(CheckOptions),
    /// Add or normalize headers in selected files.
    Format(EditOptions),
    /// Remove recognized headers from selected files.
    Remove(EditOptions),
}

#[derive(Debug, Args)]
struct CheckOptions {
    /// Fail when selected files have no rule or are not UTF-8 text.
    #[arg(long)]
    fail_if_unknown: bool,
}

#[derive(Debug, Args)]
struct EditOptions {
    /// Plan changes without writing them.
    #[arg(long)]
    dry_run: bool,

    /// Fail when selected files have no rule or are not UTF-8 text.
    #[arg(long)]
    fail_if_unknown: bool,

    /// Exit unsuccessfully when files needed changes.
    #[arg(long, action = ArgAction::Set, default_value_t = true)]
    fail_if_updated: bool,
}

fn main() -> ExitCode {
    logforth::starter_log::stderr()
        .filter(RustLogFilterBuilder::from_default_env_or("info").build())
        .apply();

    run(Command::parse()).unwrap_or_else(|error| {
        log::error!("{error:?}");
        ExitCode::from(2)
    })
}

fn run(command: Command) -> CliResult<ExitCode> {
    let config = find_config(command.config)?;
    log::debug!("loading configuration from {}", config.display());
    let engine = Engine::load(&config)
        .or_raise(|| CliError::new(format!("cannot load configuration {}", config.display())))?;
    match command.subcommand {
        SubcommandOptions::Check(options) => {
            let plan = engine
                .plan(Mode::Check)
                .or_raise(|| CliError::new("cannot analyze selected files"))?;
            emit(&plan, command.output_format)?;
            let report = plan.report();
            let failed = report.has_violations()
                || (options.fail_if_unknown && report.count(Status::Unsupported) > 0);
            Ok(policy_exit(failed))
        }
        SubcommandOptions::Format(options) => {
            edit(&engine, Mode::Format, options, command.output_format)
        }
        SubcommandOptions::Remove(options) => {
            edit(&engine, Mode::Remove, options, command.output_format)
        }
    }
}

fn edit(
    engine: &Engine,
    mode: Mode,
    options: EditOptions,
    output_format: OutputFormat,
) -> CliResult<ExitCode> {
    let plan = engine
        .plan(mode)
        .or_raise(|| CliError::new("cannot analyze selected files"))?;
    let report = if options.dry_run {
        plan.report()
    } else {
        plan.apply()
            .or_raise(|| CliError::new("cannot apply planned file edits"))?
    };
    emit(&plan, output_format)?;
    let failed = report.count(Status::Conflict) > 0
        || (options.fail_if_unknown && report.count(Status::Unsupported) > 0)
        || (options.fail_if_updated && report.changed() > 0);
    Ok(policy_exit(failed))
}

fn emit(plan: &Plan, output_format: OutputFormat) -> CliResult<()> {
    let report = plan.report();
    match output_format {
        OutputFormat::Human => emit_human(plan, &report)
            .or_raise(|| CliError::new("cannot write human-readable report"))?,
        OutputFormat::Json => {
            let mut stdout = io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &report)
                .or_raise(|| CliError::new("cannot serialize JSON report"))?;
            writeln!(stdout).or_raise(|| CliError::new("cannot write JSON report"))?;
        }
    }
    Ok(())
}

fn emit_human(plan: &Plan, report: &Report) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    for file in plan.files() {
        let label = if file.changed() {
            match (plan.mode(), file.status()) {
                (Mode::Remove, _) => "remove",
                (_, Status::Missing) => "add",
                _ => "replace",
            }
        } else {
            match file.status() {
                Status::Clean => continue,
                Status::Missing if plan.mode() == Mode::Remove => continue,
                Status::Missing => "missing",
                Status::Replaceable => "replaceable",
                Status::Conflict => "conflict",
                Status::Unsupported => "unsupported",
            }
        };
        let path = file.path().to_string_lossy().replace('\\', "/");
        writeln!(stdout, "{label:>11}  {path}")?;
    }
    writeln!(
        stdout,
        "{} files, {} changed, {} conflicts, {} unsupported",
        report.files().len(),
        report.changed(),
        report.count(Status::Conflict),
        report.count(Status::Unsupported),
    )
}

fn policy_exit(failed: bool) -> ExitCode {
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn find_config(config: Option<PathBuf>) -> CliResult<PathBuf> {
    if let Some(config) = config {
        return Ok(config);
    }

    let current_dir =
        std::env::current_dir().or_raise(|| CliError::new("cannot read the current directory"))?;
    let filenames = ["licenserc.toml", ".licenserc.toml"];
    for filename in filenames {
        let candidate = current_dir.join(filename);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    exn::bail!(CliError::new(format!(
        "cannot find {} in {}; pass --config to select another file",
        filenames.join(" or "),
        current_dir.display()
    )))
}

type CliResult<T> = exn::Result<T, CliError>;

#[derive(Debug)]
struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}
