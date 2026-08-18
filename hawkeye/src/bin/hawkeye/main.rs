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
use hawkeye::Action;
use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::Outcome;
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
    fail_on_unknown: bool,
}

#[derive(Debug, Args)]
struct EditOptions {
    /// Plan changes without writing them.
    #[arg(long)]
    dry_run: bool,

    /// Fail when selected files have no rule or are not UTF-8 text.
    #[arg(long)]
    fail_on_unknown: bool,

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
    let Command {
        config,
        output_format,
        subcommand,
    } = Command::parse();

    let config = match config {
        Some(path) => path,
        None => default_config()?,
    };
    log::debug!("loading config from {}", config.display());

    let config = Config::load(config).or_raise(|| Error::new("cannot load config"))?;
    let engine = Engine::new(config).or_raise(|| Error::new("cannot create engine"))?;

    let (report, failed) = match subcommand {
        SubcommandOptions::Check(options) => {
            let make_error = || Error::new("failed to execute check command");
            let plan = engine.plan(Action::Check).or_raise(make_error)?;
            let report = plan.report();
            let failed = report.files.iter().any(|file| match file.outcome {
                Outcome::Clean => false,
                Outcome::Add | Outcome::Replace | Outcome::Remove | Outcome::Conflict => true,
                Outcome::Unsupported => options.fail_on_unknown,
            });
            (report, failed)
        }
        SubcommandOptions::Format(options) => {
            let make_error = || Error::new("failed to execute format command");
            let plan = engine.plan(Action::Format).or_raise(make_error)?;
            if !options.dry_run {
                plan.apply().or_raise(make_error)?;
            }
            let report = plan.report();
            let failed = report.files.iter().any(|file| match file.outcome {
                Outcome::Clean => false,
                Outcome::Add | Outcome::Replace | Outcome::Remove => options.fail_on_change,
                Outcome::Conflict => true,
                Outcome::Unsupported => options.fail_on_unknown,
            });
            (report, failed)
        }
        SubcommandOptions::Remove(options) => {
            let make_error = || Error::new("failed to execute remove command");
            let plan = engine.plan(Action::Remove).or_raise(make_error)?;
            if !options.dry_run {
                plan.apply().or_raise(make_error)?;
            }
            let report = plan.report();
            let failed = report.files.iter().any(|file| match file.outcome {
                Outcome::Clean => false,
                Outcome::Add | Outcome::Replace | Outcome::Remove => options.fail_on_change,
                Outcome::Conflict => true,
                Outcome::Unsupported => options.fail_on_unknown,
            });
            (report, failed)
        }
    };

    let make_write_error = || Error::new("cannot write report to stdout");
    let mut stdout = io::stdout().lock();
    match output_format {
        OutputFormat::Json => {
            let report = serde_json::to_string_pretty(&report)
                .or_raise(|| Error::new("cannot serialize JSON report"))?;
            writeln!(stdout, "{report}").or_raise(make_write_error)?;
        }
        OutputFormat::Human => {
            for file in &report.files {
                let label = match file.outcome {
                    Outcome::Clean => continue,
                    Outcome::Add => "add",
                    Outcome::Replace => "replace",
                    Outcome::Remove => "remove",
                    Outcome::Conflict => "conflict",
                    Outcome::Unsupported => "unsupported",
                };
                writeln!(stdout, "{label:>11}  {}", file.path.display())
                    .or_raise(make_write_error)?;
            }

            let files = report.files.len();
            let mut changes = 0;
            let mut conflicts = 0;
            let mut unsupported = 0;
            for file in &report.files {
                match file.outcome {
                    Outcome::Clean => continue,
                    Outcome::Add | Outcome::Replace | Outcome::Remove => changes += 1,
                    Outcome::Conflict => conflicts += 1,
                    Outcome::Unsupported => unsupported += 1,
                }
            }
            writeln!(
                stdout,
                "{files} files, {changes} changes, {conflicts} conflicts, {unsupported} unsupported"
            )
            .or_raise(make_write_error)?;
        }
    }

    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn default_config() -> Result<PathBuf, Error> {
    let candidates = [
        PathBuf::from("licenserc.toml"),
        PathBuf::from(".licenserc.toml"),
    ];

    for path in &candidates {
        if path.is_file() {
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
