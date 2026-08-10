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

use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::ArgAction;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use hawkeye::Engine;
use hawkeye::Mode;
use hawkeye::Plan;
use hawkeye::Report;
use hawkeye::Status;
use hawkeye::config::DEFAULT_CONFIG_FILE;
use similar::TextDiff;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Command {
    #[arg(
        long,
        global = true,
        default_value = DEFAULT_CONFIG_FILE,
        help = "Configuration file; relative paths use the current directory"
    )]
    config: PathBuf,

    #[arg(long, global = true, value_enum, default_value_t = Output::Human)]
    output: Output,

    #[command(subcommand)]
    subcommand: SubcommandOptions,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Output {
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
    /// Print the edits that `format` would make.
    #[arg(long)]
    diff: bool,

    /// Fail when selected files have no rule or are not UTF-8 text.
    #[arg(long)]
    fail_if_unknown: bool,

    /// Files or directories to process; defaults to `files.root`.
    paths: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct EditOptions {
    /// Plan changes without writing them.
    #[arg(long)]
    dry_run: bool,

    /// Print unified diffs for planned changes.
    #[arg(long)]
    diff: bool,

    /// Fail when selected files have no rule or are not UTF-8 text.
    #[arg(long)]
    fail_if_unknown: bool,

    /// Exit unsuccessfully when files needed changes.
    #[arg(long, action = ArgAction::Set, default_value_t = true)]
    fail_if_updated: bool,

    /// Files or directories to process; defaults to `files.root`.
    paths: Vec<PathBuf>,
}

fn main() -> ExitCode {
    match run(Command::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("hawkeye: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(command: Command) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let engine = Engine::load(&command.config)?;
    match command.subcommand {
        SubcommandOptions::Check(options) => {
            let mode = if options.diff {
                Mode::Format
            } else {
                Mode::Check
            };
            let plan = engine.plan(mode, &options.paths)?;
            emit(&plan, command.output, options.diff)?;
            let report = plan.report();
            let failed = report.has_violations()
                || (options.fail_if_unknown && report.count(Status::Unsupported) > 0);
            Ok(policy_exit(failed))
        }
        SubcommandOptions::Format(options) => edit(&engine, Mode::Format, options, command.output),
        SubcommandOptions::Remove(options) => edit(&engine, Mode::Remove, options, command.output),
    }
}

fn edit(
    engine: &Engine,
    mode: Mode,
    options: EditOptions,
    output: Output,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let plan = engine.plan(mode, &options.paths)?;
    let report = if options.dry_run {
        plan.report()
    } else {
        plan.apply()?
    };
    emit(&plan, output, options.diff)?;
    let failed = report.count(Status::Conflict) > 0
        || (options.fail_if_unknown && report.count(Status::Unsupported) > 0)
        || (options.fail_if_updated && report.changed() > 0);
    Ok(policy_exit(failed))
}

fn emit(plan: &Plan, output: Output, show_diff: bool) -> Result<(), Box<dyn std::error::Error>> {
    if show_diff {
        let mut writer: Box<dyn Write> = match output {
            Output::Human => Box::new(io::stdout().lock()),
            Output::Json => Box::new(io::stderr().lock()),
        };
        for file in plan.files().iter().filter(|file| file.changed()) {
            let old = file.original().expect("changed files are UTF-8");
            let new = file.updated().expect("changed files have replacement text");
            let path = file.path().to_string_lossy().replace('\\', "/");
            let diff = TextDiff::from_lines(old, new)
                .unified_diff()
                .header(&format!("a/{path}"), &format!("b/{path}"))
                .to_string();
            writer.write_all(diff.as_bytes())?;
        }
    }

    let report = plan.report();
    match output {
        Output::Human => emit_human(plan, &report)?,
        Output::Json => {
            let mut stdout = io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &report)?;
            writeln!(stdout)?;
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
        writeln!(stdout, "{label:>11}  {}", file.path().display())?;
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
