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

use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use clap::Args;
use clap::Parser;
use exn::Result;
use hawkeye_fmt::document::Document;
use hawkeye_fmt::error::Error;
use hawkeye_fmt::header::matcher::HeaderMatcher;
use hawkeye_fmt::processor::check_license_header;
use hawkeye_fmt::processor::check_license_header_of_paths;
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
    #[arg(
        value_name = "PATH",
        help = "Process only these files and directories, instead of the whole baseDir"
    )]
    paths: Vec<PathBuf>,
    #[arg(
        long,
        value_name = "FILE",
        help = "Read the paths to process from FILE, one per line ('-' for stdin)"
    )]
    files_from: Option<PathBuf>,
}

impl SharedOptions {
    /// The paths to process, or `None` to process the whole `baseDir`.
    fn selected_paths(&self) -> Option<Vec<PathBuf>> {
        if self.paths.is_empty() && self.files_from.is_none() {
            return None;
        }

        let mut paths = self.paths.clone();
        if let Some(file) = self.files_from.as_ref() {
            paths.extend(read_paths_from(file));
        }
        Some(paths)
    }

    fn process<C: Callback>(&self, config: PathBuf, context: &mut C) {
        let result = match self.selected_paths() {
            None => check_license_header(config, context),
            Some(paths) => check_license_header_of_paths(config, &paths, context),
        };
        result.unwrap();
    }
}

fn read_paths_from(file: &Path) -> Vec<PathBuf> {
    fn do_read_paths_from(file: &Path) -> std::io::Result<String> {
        if file == Path::new("-") {
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content)?;
            return Ok(content);
        }
        std::fs::read_to_string(file)
    }

    let content = do_read_paths_from(file)
        .unwrap_or_else(|err| panic!("failed to read paths from file {}: {}", file.display(), err));

    // tolerate the NUL separated output of git commands, like `git diff -z --name-only`
    content
        .split(['\n', '\0'])
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
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
        help = "whether to exit with non-zero code if missing headers",
        action = clap::ArgAction::Set,
        default_value_t = true
    )]
    fail_if_missing: bool,
}

#[derive(Serialize)]
struct CheckContext {
    unknown: Vec<String>,
    missing: Vec<String>,
}

impl Callback for CheckContext {
    fn on_unknown(&mut self, path: &Path) {
        self.unknown.push(path.display().to_string());
    }

    fn on_matched(&mut self, _: &HeaderMatcher, _: Document) -> Result<(), Error> {
        Ok(())
    }

    fn on_not_matched(&mut self, _: &HeaderMatcher, document: Document) -> Result<(), Error> {
        self.missing.push(document.filepath.display().to_string());
        Ok(())
    }
}

impl CommandCheck {
    fn run(mut self) {
        let config = self.shared.config.take().unwrap_or_else(default_config);
        let mut context = CheckContext {
            unknown: vec![],
            missing: vec![],
        };
        self.shared.process(config, &mut context);

        let mut failed = check_unknown_files(&context.unknown, self.shared.fail_if_unknown);
        if !context.missing.is_empty() {
            log::error!("Found header missing in files: {:?}", context.missing);
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
}

impl Callback for FormatContext {
    fn on_unknown(&mut self, path: &Path) {
        self.unknown.push(path.display().to_string());
    }

    fn on_matched(&mut self, _: &HeaderMatcher, _: Document) -> Result<(), Error> {
        Ok(())
    }

    fn on_not_matched(&mut self, header: &HeaderMatcher, mut doc: Document) -> Result<(), Error> {
        if doc.header_detected() {
            doc.remove_header();
            doc.update_header(header)?;
            self.updated
                .push(format!("{}=replaced", doc.filepath.display()));
        } else {
            doc.update_header(header)?;
            self.updated
                .push(format!("{}=added", doc.filepath.display()));
        }

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
    fn run(mut self) {
        let config = self.shared.config.take().unwrap_or_else(default_config);
        let mut context = FormatContext {
            dry_run: self.shared_edit.dry_run,
            unknown: vec![],
            updated: vec![],
        };
        self.shared.process(config, &mut context);

        let mut failed = check_unknown_files(&context.unknown, self.shared.fail_if_unknown);
        if !context.updated.is_empty() {
            log::info!(
                "Updated header for files (dryRun={}): {:?}",
                self.shared_edit.dry_run,
                context.updated
            );
            failed |= self.shared_edit.fail_if_updated;
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
}

impl RemoveContext {
    fn remove(&mut self, doc: &mut Document) -> Result<(), Error> {
        if !doc.header_detected() {
            return Ok(());
        }
        doc.remove_header();
        self.removed
            .push(format!("{}=removed", doc.filepath.display()));
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
    fn run(mut self) {
        let config = self.shared.config.take().unwrap_or_else(default_config);
        let mut context = RemoveContext {
            dry_run: self.shared_edit.dry_run,
            unknown: vec![],
            removed: vec![],
        };
        self.shared.process(config, &mut context);

        let mut failed = check_unknown_files(&context.unknown, self.shared.fail_if_unknown);
        if !context.removed.is_empty() {
            log::info!(
                "Removed header for files (dryRun={}): {:?}",
                self.shared_edit.dry_run,
                context.removed
            );
            failed |= self.shared_edit.fail_if_updated;
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
