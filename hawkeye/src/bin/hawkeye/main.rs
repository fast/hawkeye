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

use std::env;
#[cfg(unix)]
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
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
    #[command(flatten)]
    selection: SelectionOptions,

    /// Fail when selected files have no rule or are not UTF-8 text.
    #[arg(long)]
    fail_on_unknown: bool,
}

#[derive(Debug, Args)]
struct EditOptions {
    #[command(flatten)]
    selection: SelectionOptions,

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

#[derive(Debug, Args)]
struct SelectionOptions {
    /// Files and directories to process.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Read paths to process from a file, or stdin with `-`.
    #[arg(long, value_name = "FILE")]
    files_from: Option<PathBuf>,
}

impl SelectionOptions {
    fn resolve(mut self) -> Result<Option<Vec<PathBuf>>, Error> {
        if self.paths.is_empty() && self.files_from.is_none() {
            return Ok(None);
        }

        if let Some(path) = self.files_from {
            self.paths.extend(read_paths_from(&path)?);
        }

        let current_dir = env::current_dir()
            .map_err(|err| Error::new(format!("cannot resolve the current directory: {err}")))?;
        for path in &mut self.paths {
            if path.is_relative() {
                *path = current_dir.join(&*path);
            }
        }
        Ok(Some(self.paths))
    }
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
        subcommand,
    } = Command::parse();

    let config = match config {
        Some(path) => path,
        None => default_config()?,
    };
    log::debug!("loading config from {}", config.display());

    let config = Config::load(config).or_raise(|| Error::new("cannot load config"))?;
    let engine = Engine::new(config).or_raise(|| Error::new("cannot create engine"))?;

    let (report, fail_on_change, fail_on_unknown) = match subcommand {
        SubcommandOptions::Check(options) => {
            let paths = options.selection.resolve()?;
            let make_error = || Error::new("failed to execute check command");
            let report = match paths.as_deref() {
                Some(paths) => engine.check_paths(paths),
                None => engine.check(),
            }
            .or_raise(make_error)?;
            (report, true, options.fail_on_unknown)
        }
        SubcommandOptions::Format(options) => {
            let paths = options.selection.resolve()?;
            let make_error = || Error::new("failed to execute format command");
            let edits = match paths.as_deref() {
                Some(paths) => engine.format_paths(paths),
                None => engine.format(),
            }
            .or_raise(make_error)?;
            let report = if options.dry_run {
                edits.into_report()
            } else {
                edits.apply().or_raise(make_error)?
            };
            (report, options.fail_on_change, options.fail_on_unknown)
        }
        SubcommandOptions::Remove(options) => {
            let paths = options.selection.resolve()?;
            let make_error = || Error::new("failed to execute remove command");
            let edits = match paths.as_deref() {
                Some(paths) => engine.remove_paths(paths),
                None => engine.remove(),
            }
            .or_raise(make_error)?;
            let report = if options.dry_run {
                edits.into_report()
            } else {
                edits.apply().or_raise(make_error)?
            };
            (report, options.fail_on_change, options.fail_on_unknown)
        }
    };

    let failed = report.files.iter().any(|file| match file.outcome {
        FileOutcome::Clean => false,
        FileOutcome::Add | FileOutcome::Replace | FileOutcome::Remove => fail_on_change,
        FileOutcome::Conflict => true,
        FileOutcome::Unsupported => fail_on_unknown,
    });

    match output_format {
        OutputFormat::Json => {
            let mut output = serde_json::to_vec_pretty(&report)
                .or_raise(|| Error::new("cannot serialize JSON report"))?;
            output.push(b'\n');

            let make_error = || Error::new("cannot output JSON report");
            let mut stdout = io::stdout().lock();
            stdout.write_all(&output).or_raise(make_error)?;
        }
        OutputFormat::Human => {
            let make_error = || Error::new("cannot output human-readable report");

            let mut stdout = io::stdout().lock();
            for file in &report.files {
                let label = match file.outcome {
                    FileOutcome::Clean => continue,
                    FileOutcome::Add => "add",
                    FileOutcome::Replace => "replace",
                    FileOutcome::Remove => "remove",
                    FileOutcome::Conflict => "conflict",
                    FileOutcome::Unsupported => "unsupported",
                };
                writeln!(stdout, "{label:>11}  {}", file.path.display()).or_raise(make_error)?;
            }

            let files = report.files.len();
            let mut changes = 0;
            let mut conflicts = 0;
            let mut unsupported = 0;
            for file in &report.files {
                match file.outcome {
                    FileOutcome::Clean => continue,
                    FileOutcome::Add | FileOutcome::Replace | FileOutcome::Remove => changes += 1,
                    FileOutcome::Conflict => conflicts += 1,
                    FileOutcome::Unsupported => unsupported += 1,
                }
            }
            let file_label = if files == 1 { "file" } else { "files" };
            let change_label = if changes == 1 { "change" } else { "changes" };
            let conflict_label = if conflicts == 1 {
                "conflict"
            } else {
                "conflicts"
            };
            writeln!(
                stdout,
                "{files} {file_label}, {changes} {change_label}, {conflicts} {conflict_label}, {unsupported} unsupported"
            )
            .or_raise(make_error)?;
        }
    }

    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
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

fn read_paths_from(path: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut content = Vec::new();
    if path == Path::new("-") {
        io::stdin()
            .read_to_end(&mut content)
            .map_err(|err| Error::new(format!("cannot read paths from stdin: {err}")))?;
    } else {
        content = fs::read(path).map_err(|err| {
            Error::new(format!("cannot read paths from {}: {err}", path.display()))
        })?;
    }

    let nul_separated = content.contains(&b'\0');
    let mut paths = Vec::new();
    for mut value in content.split(|byte| {
        if nul_separated {
            *byte == b'\0'
        } else {
            *byte == b'\n'
        }
    }) {
        if !nul_separated && value.last() == Some(&b'\r') {
            value = &value[..value.len() - 1];
        }
        if !value.is_empty() {
            paths.push(path_from_bytes(value.to_vec())?);
        }
    }
    Ok(paths)
}

#[cfg(unix)]
fn path_from_bytes(value: Vec<u8>) -> Result<PathBuf, Error> {
    Ok(PathBuf::from(OsString::from_vec(value)))
}

#[cfg(not(unix))]
fn path_from_bytes(value: Vec<u8>) -> Result<PathBuf, Error> {
    String::from_utf8(value)
        .map(PathBuf::from)
        .map_err(|err| Error::new(format!("path list contains non-UTF-8 data: {err}")))
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
