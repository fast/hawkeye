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

//! The HawkEye command-line interface.

use std::error::Error;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use hawkeye::DEFAULT_CONFIG_FILE;
use hawkeye::Engine;
use hawkeye::Mode;
use hawkeye::Plan;
use hawkeye::Report;
use hawkeye::Status;
use serde::Serialize;
use similar::TextDiff;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Path to the v7 configuration file.
    #[arg(long, global = true, default_value = DEFAULT_CONFIG_FILE)]
    config: PathBuf,

    /// Selects human-readable or JSON output.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    output_format: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Reports non-compliant license headers without changing files.
    Check {
        /// Prints the changes that `hawkeye format` would make.
        #[arg(long)]
        diff: bool,
    },
    /// Adds or replaces license headers when the edit is safe.
    Format(EditArgs),
    /// Removes license headers when their exact range is known.
    Remove(EditArgs),
}

#[derive(Debug, Args)]
struct EditArgs {
    /// Plans changes without writing source files.
    #[arg(long)]
    dry_run: bool,

    /// Prints a unified diff for every planned change.
    #[arg(long)]
    diff: bool,
}

#[derive(Serialize)]
struct CommandOutput<'report> {
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
    changed: usize,
    report: &'report Report,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            let stderr = io::stderr();
            let _ = writeln!(stderr.lock(), "error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, Box<dyn Error>> {
    let diff = match &cli.command {
        Command::Check { diff } => *diff,
        Command::Format(args) | Command::Remove(args) => args.diff,
    };
    validate_output_options(cli.output_format, diff)?;

    let engine = Engine::load(&cli.config)?;
    match cli.command {
        Command::Check { diff } => {
            let plan = engine.plan(Mode::Format)?;
            finish_check(plan, cli.output_format, diff)
        }
        Command::Format(args) => finish_edit(
            engine.plan(Mode::Format)?,
            "format",
            cli.output_format,
            args,
        ),
        Command::Remove(args) => finish_edit(
            engine.plan(Mode::Remove)?,
            "remove",
            cli.output_format,
            args,
        ),
    }
}

fn finish_check(
    plan: Plan,
    output_format: OutputFormat,
    show_diff: bool,
) -> Result<ExitCode, Box<dyn Error>> {
    if show_diff {
        print_diffs(&plan)?;
    }
    let changed = change_count(&plan);
    let report = plan.report();
    print_report(
        output_format,
        &CommandOutput {
            command: "check",
            dry_run: None,
            changed,
            report: &report,
        },
    )?;
    Ok(policy_exit(report.has_violations()))
}

fn finish_edit(
    plan: Plan,
    command: &'static str,
    output_format: OutputFormat,
    args: EditArgs,
) -> Result<ExitCode, Box<dyn Error>> {
    if args.diff {
        print_diffs(&plan)?;
    }
    let changed = change_count(&plan);
    let report = plan.report();
    let has_conflict = report.count(Status::Conflict) > 0;

    if !args.dry_run {
        plan.apply()?;
    }
    print_report(
        output_format,
        &CommandOutput {
            command,
            dry_run: Some(args.dry_run),
            changed,
            report: &report,
        },
    )?;

    Ok(policy_exit(has_conflict || (args.dry_run && changed > 0)))
}

fn print_report(
    output_format: OutputFormat,
    output: &CommandOutput<'_>,
) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    match output_format {
        OutputFormat::Json => {
            serde_json::to_writer_pretty(&mut writer, output)?;
            writeln!(writer)?;
        }
        OutputFormat::Human => {
            for file in output
                .report
                .files()
                .iter()
                .filter(|file| file.status() != Status::Clean)
            {
                writeln!(writer, "{:<11} {}", file.status(), file.path().display())?;
            }
            writeln!(
                writer,
                "{} files, {} changed, {} conflicts, {} unsupported{}",
                output.report.files().len(),
                output.changed,
                output.report.count(Status::Conflict),
                output.report.count(Status::Unsupported),
                if output.dry_run == Some(true) {
                    " (dry run)"
                } else {
                    ""
                }
            )?;
        }
    }
    Ok(())
}

fn print_diffs(plan: &Plan) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    for file in plan.files().iter().filter(|file| file.edit().is_some()) {
        let original = file
            .original()
            .expect("planned edits only exist for UTF-8 files");
        let updated = file
            .updated()?
            .expect("planned edits always produce UTF-8 files");
        let path = diff_path(file.path());
        let diff = TextDiff::from_lines(original, &updated)
            .unified_diff()
            .header(&format!("a/{path}"), &format!("b/{path}"))
            .to_string();
        writer.write_all(diff.as_bytes())?;
    }
    Ok(())
}

fn diff_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn change_count(plan: &Plan) -> usize {
    plan.files()
        .iter()
        .filter(|file| file.edit().is_some())
        .count()
}

fn validate_output_options(output_format: OutputFormat, diff: bool) -> Result<(), io::Error> {
    if output_format == OutputFormat::Json && diff {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--diff cannot be combined with --output-format json",
        ));
    }
    Ok(())
}

fn policy_exit(failed: bool) -> ExitCode {
    ExitCode::from(u8::from(failed))
}
