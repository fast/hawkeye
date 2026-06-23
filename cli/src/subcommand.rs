// Copyright 2024 tison <wander4096@gmail.com>
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

use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use clap::Args;
use clap::Parser;
use exn::Result;
use hawkeye_fmt::config::ExistingStrategy;
use hawkeye_fmt::document::Document;
use hawkeye_fmt::error::Error;
use hawkeye_fmt::header::matcher::HeaderMatcher;
use hawkeye_fmt::processor::check_license_header;
use hawkeye_fmt::processor::Callback;
use serde::Serialize;

#[derive(Parser)]
pub enum SubCommand {
    #[clap(name = "check", about = "check license header")]
    Check(CommandCheck),
    #[clap(name = "format", about = "format license header")]
    Format(CommandFormat),
    #[clap(name = "remove", about = "remove license header")]
    Remove(CommandRemove),
}

#[derive(Args)]
struct SharedOptions {
    #[arg(long, help = "path to the config file")]
    config: Option<PathBuf>,
    #[arg(
        short = 'o',
        long = "output",
        help = "Write the output as JSON object to the specified file"
    )]
    output_file: Option<PathBuf>,
    #[arg(long, help = "fail if process unknown files", default_value_t = false)]
    fail_if_unknown: bool,
}

#[derive(Args)]
struct SharedEditOptions {
    #[arg(long, help = "whether update file in place", default_value_t = false)]
    dry_run: bool,
    #[arg(
        long,
        help = "whether to exit with non-zero code if files updated",
        action = clap::ArgAction::Set,
        default_value_t = true
    )]
    fail_if_updated: bool,
}

impl SubCommand {
    pub fn run(self) {
        match self {
            SubCommand::Check(cmd) => cmd.run(),
            SubCommand::Format(cmd) => cmd.run(),
            SubCommand::Remove(cmd) => cmd.run(),
        }
    }
}

#[derive(Parser)]
pub struct CommandCheck {
    #[command(flatten)]
    shared: SharedOptions,
    #[arg(
        long,
        help = "whether to exit with non-zero code if headers are missing or non-matching (foreign)",
        action = clap::ArgAction::Set,
        default_value_t = true
    )]
    fail_if_missing: bool,
}

#[derive(Serialize)]
struct CheckContext {
    unknown: Vec<String>,
    missing: Vec<String>,
    foreign: Vec<String>,
}

impl Callback for CheckContext {
    fn on_unknown(&mut self, path: &Path) {
        self.unknown.push(path.display().to_string());
    }

    fn on_matched(&mut self, _: &HeaderMatcher, _: Document) -> Result<(), Error> {
        Ok(())
    }

    fn on_not_matched(&mut self, _: &HeaderMatcher, document: Document) -> Result<(), Error> {
        let path = document.filepath.display().to_string();
        // A file that already carries a non-matching header (a stale one in a recognized
        // style, or a notice in a style this rule does not list) is reported distinctly from
        // a file that has no header at all.
        if document.has_existing_header() {
            self.foreign.push(path);
        } else {
            self.missing.push(path);
        }
        Ok(())
    }
}

impl CommandCheck {
    fn run(self) {
        let config = self.shared.config.unwrap_or_else(default_config);
        let mut context = CheckContext {
            unknown: vec![],
            missing: vec![],
            foreign: vec![],
        };
        run_processor(config, &mut context);

        let mut failed = check_unknown_files(&context.unknown, self.shared.fail_if_unknown);
        if !context.missing.is_empty() {
            log::error!("Found header missing in files: {:?}", context.missing);
            failed |= self.fail_if_missing;
        }
        if !context.foreign.is_empty() {
            log::error!(
                "Found files with a non-matching existing header (different style or outdated content): {:?}",
                context.foreign
            );
            failed |= self.fail_if_missing;
        }
        if let Some(f) = self.shared.output_file {
            write_to_file(&f, &context);
        }
        if failed {
            std::process::exit(1);
        }
        log::info!("No missing header file has been found.");
    }
}

#[derive(Parser)]
pub struct CommandFormat {
    #[command(flatten)]
    shared: SharedOptions,
    #[command(flatten)]
    shared_edit: SharedEditOptions,
}

#[derive(Serialize)]
struct FormatContext {
    dry_run: bool,
    unknown: Vec<String>,
    updated: Vec<String>,
    skipped: Vec<String>,
    foreign: Vec<String>,
    conflict: Vec<String>,
}

impl Callback for FormatContext {
    fn on_unknown(&mut self, path: &Path) {
        self.unknown.push(path.display().to_string());
    }

    fn on_matched(&mut self, _: &HeaderMatcher, _: Document) -> Result<(), Error> {
        Ok(())
    }

    fn on_not_matched(&mut self, header: &HeaderMatcher, mut doc: Document) -> Result<(), Error> {
        let path = doc.filepath.display().to_string();

        // `kind` is set only when the file is actually rewritten (added/replaced); skip and
        // error outcomes leave it untouched. `error` must not return Err: `run_processor`
        // aggregates conflicts and exits(1), like the missing/updated paths.
        let strategy = doc.existing_strategy();
        let kind = match strategy {
            ExistingStrategy::Replace => {
                // Strip ALL recognized header blocks (preferred style + every listed foreign
                // style, in either stacking order), then insert exactly one preferred header.
                // Re-check `looks_licensed` AFTER stripping: a notice in an UNLISTED style is not
                // recognized, so it survives the strip; inserting on top of it would re-stack the
                // #210 duplicate over a survivor. If anything still looks licensed (a surviving
                // unlisted notice, or a stray keyword), leave the file for a human - a controlled
                // failure beats a silent duplicate.
                let removed = doc.remove_existing_headers();
                if doc.looks_licensed() {
                    log::warn!(
                        "{path}: looks licensed (unlisted comment style or a stray keyword); left unchanged"
                    );
                    self.foreign.push(path);
                    return Ok(());
                }
                doc.update_header(header)?;
                if removed {
                    "replaced"
                } else {
                    "added"
                }
            }
            // Skip and Error differ only in the bucket: Error -> conflict (exits 1 in
            // CommandFormat::run), Skip -> skipped (warn only).
            ExistingStrategy::Skip | ExistingStrategy::Error => {
                if doc.has_existing_header() {
                    let bucket = if strategy == ExistingStrategy::Error {
                        &mut self.conflict
                    } else {
                        &mut self.skipped
                    };
                    bucket.push(path);
                    return Ok(());
                }
                doc.update_header(header)?;
                "added"
            }
        };

        self.updated.push(format!("{path}={kind}"));

        if self.dry_run {
            let mut extension = doc.filepath.extension().unwrap_or_default().to_os_string();
            extension.push(".formatted");
            let copied = doc.filepath.with_extension(extension);
            doc.save_to(copied)
        } else {
            doc.save()
        }
    }
}

impl CommandFormat {
    fn run(self) {
        let config = self.shared.config.unwrap_or_else(default_config);
        let mut context = FormatContext {
            dry_run: self.shared_edit.dry_run,
            unknown: vec![],
            updated: vec![],
            skipped: vec![],
            foreign: vec![],
            conflict: vec![],
        };
        run_processor(config, &mut context);

        let mut failed = check_unknown_files(&context.unknown, self.shared.fail_if_unknown);
        if !context.updated.is_empty() {
            log::info!(
                "Updated header for files (dryRun={}): {:?}",
                self.shared_edit.dry_run,
                context.updated
            );
            failed |= self.shared_edit.fail_if_updated;
        }
        if !context.skipped.is_empty() {
            log::warn!(
                "Skipped files with an existing header: {:?}",
                context.skipped
            );
        }
        if !context.foreign.is_empty() {
            // existingStrategy=replace found a license-looking header it could not normalize to
            // the preferred style (an unlisted comment style, or a stray keyword). Leaving it
            // would silently exit 0 with the file un-normalized, so this is a controlled failure.
            // Unlike `skipped` (the user's explicit opt-out), `foreign` is not a chosen outcome.
            log::warn!(
                "Files that look licensed (unlisted comment style or a stray keyword); left unchanged: {:?}",
                context.foreign
            );
            failed = true;
        }
        if !context.conflict.is_empty() {
            log::error!(
                "Existing headers conflict with existingStrategy=error: {:?}",
                context.conflict
            );
            failed = true;
        }
        if let Some(f) = self.shared.output_file {
            write_to_file(&f, &context);
        }
        if failed {
            std::process::exit(1);
        }
        log::info!("All files have proper header.");
    }
}

#[derive(Parser)]
pub struct CommandRemove {
    #[command(flatten)]
    shared: SharedOptions,
    #[command(flatten)]
    shared_edit: SharedEditOptions,
}

#[derive(Serialize)]
struct RemoveContext {
    dry_run: bool,
    unknown: Vec<String>,
    removed: Vec<String>,
    foreign: Vec<String>,
}

impl RemoveContext {
    fn remove(&mut self, doc: &mut Document) -> Result<(), Error> {
        let path = doc.filepath.display().to_string();
        // Remove a header in the preferred style, or failing that any other listed style.
        let removed = if doc.header_detected() {
            doc.remove_header();
            true
        } else {
            doc.remove_foreign_header()
        };

        if !removed {
            if doc.looks_licensed() {
                log::warn!(
                    "{path}: looks licensed (unlisted comment style or a stray keyword); not removed"
                );
                self.foreign.push(path);
            }
            return Ok(());
        }

        self.removed.push(path);
        if self.dry_run {
            let mut extension = doc.filepath.extension().unwrap_or_default().to_os_string();
            extension.push(".removed");
            let copied = doc.filepath.with_extension(extension);
            doc.save_to(copied)
        } else {
            doc.save()
        }
    }
}

impl Callback for RemoveContext {
    fn on_unknown(&mut self, path: &Path) {
        self.unknown.push(path.display().to_string());
    }

    fn on_matched(&mut self, _: &HeaderMatcher, mut doc: Document) -> Result<(), Error> {
        self.remove(&mut doc)
    }

    fn on_not_matched(&mut self, _: &HeaderMatcher, mut doc: Document) -> Result<(), Error> {
        self.remove(&mut doc)
    }
}

impl CommandRemove {
    fn run(self) {
        let config = self.shared.config.unwrap_or_else(default_config);
        let mut context = RemoveContext {
            dry_run: self.shared_edit.dry_run,
            unknown: vec![],
            removed: vec![],
            foreign: vec![],
        };
        run_processor(config, &mut context);

        let mut failed = check_unknown_files(&context.unknown, self.shared.fail_if_unknown);
        if !context.removed.is_empty() {
            log::info!(
                "Removed header for files (dryRun={}): {:?}",
                self.shared_edit.dry_run,
                context.removed
            );
            failed |= self.shared_edit.fail_if_updated;
        }
        if !context.foreign.is_empty() {
            log::warn!(
                "Files that look licensed (unlisted comment style or a stray keyword); not removed: {:?}",
                context.foreign
            );
        }
        if let Some(f) = self.shared.output_file {
            write_to_file(&f, &context);
        }
        if failed {
            std::process::exit(1);
        }
        log::info!("No file has been removed header.");
    }
}

/// Run the processor, surfacing a config/IO error as a clean stderr message + `exit(1)`
/// rather than an `unwrap` panic that buries it (e.g. the 6.x migration hint) under a
/// backtrace. Matches the missing/conflict paths, which also log and `exit(1)`.
fn run_processor<C: Callback>(config: PathBuf, callback: &mut C) {
    if let Err(err) = check_license_header(config, callback) {
        log::error!("{err:?}");
        std::process::exit(1);
    }
}

fn write_to_file(path: &Path, result: impl Serialize) {
    fn do_write_to_file(path: &Path, result: impl Serialize) -> std::io::Result<()> {
        let mut file = std::fs::File::create(path)?;
        serde_json::to_writer(&file, &result)?;
        file.flush()
    }

    do_write_to_file(path, result)
        .unwrap_or_else(|err| panic!("failed to write output to file {}: {}", path.display(), err));
}

fn check_unknown_files(unknown: &[String], fail_if_unknown: bool) -> bool {
    if !unknown.is_empty() {
        if fail_if_unknown {
            log::error!("Processing unknown files: {unknown:?}");
            return true;
        } else {
            log::warn!("Processing unknown files: {unknown:?}");
        }
    }
    false
}

fn default_config() -> PathBuf {
    let candidates = [
        PathBuf::from("licenserc.toml"),
        PathBuf::from(".licenserc.toml"),
    ];

    for path in &candidates {
        if path.exists() {
            return path.clone();
        }
    }

    panic!(
        "cannot find config file in any of the default locations: {:?}",
        candidates.iter().map(|p| p.display()).collect::<Vec<_>>()
    );
}
