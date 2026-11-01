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

mod output;

use std::env;
use std::fmt;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use exn::Exn;
use exn::Frame;
use exn::Result;
use exn::ResultExt;
use exn::bail;
use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::FileOutcome;
use hawkeye::Scope;
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
    /// Files and directories to process.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Fail when selected files have no rule or are not UTF-8 text.
    #[arg(long)]
    fail_on_unknown: bool,
}

#[derive(Debug, Args)]
struct EditOptions {
    /// Files and directories to process.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Show changes without writing them.
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
        .filter(RustLogFilterBuilder::from_default_env_or("warn").build())
        .apply();

    do_main().unwrap_or_else(|err| {
        emit_error(&err);
        ExitCode::from(2)
    })
}

fn do_main() -> Result<ExitCode, Error> {
    let Command {
        config,
        output_format,
        mut subcommand,
    } = Command::parse();

    let config = match config {
        Some(path) => path,
        None => default_config()?,
    };
    log::debug!("loading config from {}", config.display());

    let config = Config::load(config).or_raise(|| Error::new("cannot load config"))?;
    let engine = Engine::new(config).or_raise(|| Error::new("cannot create engine"))?;

    resolve_paths(match &mut subcommand {
        SubcommandOptions::Check(options) => &mut options.paths,
        SubcommandOptions::Format(options) => &mut options.paths,
        SubcommandOptions::Remove(options) => &mut options.paths,
    })?;
    let scope = {
        let paths = match &subcommand {
            SubcommandOptions::Check(options) => &options.paths,
            SubcommandOptions::Format(options) => &options.paths,
            SubcommandOptions::Remove(options) => &options.paths,
        };
        if paths.is_empty() {
            Scope::All
        } else {
            Scope::Paths(paths)
        }
    };
    let (report, edits, fail_on_change, fail_on_unknown) = match &subcommand {
        SubcommandOptions::Check(options) => {
            let make_error = || Error::new("failed to execute check command");
            let report = engine.check(scope).or_raise(make_error)?;
            (report, None, true, options.fail_on_unknown)
        }
        SubcommandOptions::Format(options) => {
            let make_error = || Error::new("failed to execute format command");
            let edits = engine.format(scope).or_raise(make_error)?;
            let report = edits.report().clone();
            let edits = (!options.dry_run).then_some(edits);
            (
                report,
                edits,
                options.fail_on_change,
                options.fail_on_unknown,
            )
        }
        SubcommandOptions::Remove(options) => {
            let make_error = || Error::new("failed to execute remove command");
            let edits = engine.remove(scope).or_raise(make_error)?;
            let report = edits.report().clone();
            let edits = (!options.dry_run).then_some(edits);
            (
                report,
                edits,
                options.fail_on_change,
                options.fail_on_unknown,
            )
        }
    };

    let failed = report.files.iter().any(|file| match file.outcome {
        FileOutcome::Clean => false,
        FileOutcome::Add | FileOutcome::Replace | FileOutcome::Remove => fail_on_change,
        FileOutcome::Conflict => true,
        FileOutcome::Unsupported => fail_on_unknown,
    });

    let output = output_format.render(&report)?;
    if let Some(edits) = edits {
        edits
            .apply()
            .or_raise(|| Error::new("cannot apply header edits"))?;
    }
    io::stdout()
        .lock()
        .write_all(&output)
        .or_raise(|| Error::new("cannot output report"))?;

    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn resolve_paths(paths: &mut [PathBuf]) -> Result<(), Error> {
    if paths.iter().all(|path| path.is_absolute()) {
        return Ok(());
    }

    let current_dir = env::current_dir()
        .or_raise(|| Error::new("cannot resolve the current working directory"))?;
    for path in paths {
        if path.is_relative() {
            *path = current_dir.join(&*path);
        }
    }
    Ok(())
}

fn emit_error(err: &Exn<Error>) {
    fn write_causes(writer: &mut impl Write, frame: &Frame, depth: usize) -> io::Result<()> {
        for cause in frame.children() {
            for _ in 0..depth {
                writer.write_all(b"  ")?;
            }
            writeln!(writer, "caused by: {}", cause.error())?;
            write_causes(writer, cause, depth + 1)?;
        }
        Ok(())
    }

    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "error: {}", err.frame().error());
    let _ = write_causes(&mut stderr, err.frame(), 1);
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
