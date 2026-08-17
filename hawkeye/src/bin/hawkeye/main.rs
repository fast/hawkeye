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

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use exn::Result;
use exn::ResultExt;
use exn::bail;
use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::Mode;
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

    /// Exit unsuccessfully if this command changes any files.
    #[arg(long)]
    fail_on_change: bool,
}

fn main() -> ExitCode {
    logforth::starter_log::stderr()
        .filter(RustLogFilterBuilder::from_default_env_or("info").build())
        .apply();

    do_main().unwrap_or_else(|err| {
        log::error!("{err:?}");
        ExitCode::from(2)
    })
}

fn do_main() -> Result<ExitCode, Error> {
    let cmd = Command::parse();
    let config = match cmd.config {
        Some(path) => path,
        None => default_config()?,
    };
    log::debug!("loading config from {}", config.display());

    let config = Config::load(config).or_raise(|| Error::new("cannot load config"))?;
    let engine = Engine::new(config).or_raise(|| Error::new("cannot initialize HawkEye"))?;

    match cmd.subcommand {
        SubcommandOptions::Check(options) => {
            let plan = engine
                .plan(Mode::Check)
                .or_raise(|| Error::new("cannot analyze selected files"))?;
            let report = plan.report();
            emit(Mode::Check, &report, cmd.output_format)?;
            let failed = report.has_violations()
                || (options.fail_if_unknown && report.count(Status::Unsupported) > 0);
            Ok(policy_exit(failed))
        }
        SubcommandOptions::Format(options) => {
            edit(&engine, Mode::Format, options, cmd.output_format)
        }
        SubcommandOptions::Remove(options) => {
            edit(&engine, Mode::Remove, options, cmd.output_format)
        }
    }
}

fn edit(
    engine: &Engine,
    mode: Mode,
    options: EditOptions,
    output_format: OutputFormat,
) -> Result<ExitCode, Error> {
    let plan = engine
        .plan(mode)
        .or_raise(|| Error::new("cannot analyze selected files"))?;
    if !options.dry_run {
        plan.apply()
            .or_raise(|| Error::new("cannot apply planned file edits"))?;
    }
    let report = plan.report();
    emit(mode, &report, output_format)?;
    let failed = report.count(Status::Conflict) > 0
        || (options.fail_if_unknown && report.count(Status::Unsupported) > 0)
        || (options.fail_on_change && report.changed() > 0);
    Ok(policy_exit(failed))
}

fn emit(mode: Mode, report: &Report, output_format: OutputFormat) -> Result<(), Error> {
    match output_format {
        OutputFormat::Human => emit_human(mode, report)
            .or_raise(|| Error::new("cannot write human-readable report"))?,
        OutputFormat::Json => {
            let mut stdout = io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &report)
                .or_raise(|| Error::new("cannot serialize JSON report"))?;
            writeln!(stdout).or_raise(|| Error::new("cannot write JSON report"))?;
        }
    }
    Ok(())
}

fn emit_human(mode: Mode, report: &Report) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    for file in report.files() {
        let label = if file.changed {
            match (mode, file.status) {
                (Mode::Remove, _) => "remove",
                (_, Status::Missing) => "add",
                _ => "replace",
            }
        } else {
            match file.status {
                Status::Clean => continue,
                Status::Missing if mode == Mode::Remove => continue,
                Status::Missing => "missing",
                Status::Replaceable => "replaceable",
                Status::Conflict => "conflict",
                Status::Unsupported => "unsupported",
            }
        };
        let path = file.path.to_string_lossy().replace('\\', "/");
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

fn default_config() -> Result<PathBuf, Error> {
    let candidates = [
        PathBuf::from("licenserc.toml"),
        PathBuf::from(".licenserc.toml"),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    bail!(Error::new(format!(
        "cannot find a config file in any of the default locations: {:?}",
        candidates.iter().map(|p| p.display()).collect::<Vec<_>>()
    )));
}

#[derive(Debug)]
struct Error(String);

impl Error {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}
