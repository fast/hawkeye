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

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::Error;
use crate::FileAttrs;
use crate::ResolvedConfig;
use crate::Result;
use crate::analyze::analyze;
use crate::attrs::FileAttrsResolver;
use crate::discovery::discover;
use crate::git::GitRepo;
use crate::report::FileOutcome;
use crate::report::Mode;
use crate::report::Report;
use crate::report::Status;
use crate::writer::write_atomic;

/// The reusable library entry point for one resolved HawkEye configuration.
pub struct Engine {
    config: ResolvedConfig,
}

impl Engine {
    /// Creates an engine from an already resolved configuration.
    pub fn new(config: ResolvedConfig) -> Self {
        Self { config }
    }

    /// Loads `licenserc.toml` and creates an engine.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        ResolvedConfig::load(path).map(Self::new)
    }

    /// Returns the resolved configuration used by this engine.
    pub fn config(&self) -> &ResolvedConfig {
        &self.config
    }

    /// Discovers and analyzes files without modifying the filesystem.
    pub fn plan(&self, mode: Mode) -> Result<Plan> {
        let git = self.config.git();
        let repo = GitRepo::discover(self.config.root(), git.ignore().combine(git.file_attrs()))?;
        let paths = discover(&self.config, repo.as_ref())?;
        let attrs = FileAttrsResolver::new(&paths, git.file_attrs(), repo.as_ref())?;
        let mut files = Vec::with_capacity(paths.len());

        for path in paths {
            let relative = path
                .strip_prefix(self.config.root())
                .expect("discovery only returns paths inside files.root")
                .to_path_buf();
            if self.config.rule_for(&relative).is_none() {
                files.push(PlannedFile::unsupported(path, relative));
                continue;
            }

            let original = fs::read(&path).map_err(|source| Error::io("read", &path, source))?;
            let Ok(input) = std::str::from_utf8(&original) else {
                files.push(PlannedFile::unsupported(path, relative));
                continue;
            };
            let file_attrs = attrs.for_file(&path)?;
            let header = self.config.render_header(&file_attrs)?;
            let analysis = analyze(&self.config, &relative, input, &header, mode);
            let updated = analysis
                .edit
                .as_ref()
                .map(|edit| edit.apply(input))
                .transpose()?
                .filter(|output| output.as_bytes() != original)
                .map(String::into_bytes);
            files.push(PlannedFile {
                absolute_path: path,
                relative_path: relative,
                status: analysis.status,
                original: Some(original),
                updated,
                file_attrs: Some(file_attrs),
            });
        }

        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(Plan { mode, files })
    }
}

/// A complete, deterministic operation plan produced before any file is written.
pub struct Plan {
    mode: Mode,
    files: Vec<PlannedFile>,
}

impl Plan {
    /// Returns the operation represented by this plan.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Returns every selected file in path order.
    pub fn files(&self) -> &[PlannedFile] {
        &self.files
    }

    /// Builds the serializable report for this plan.
    pub fn report(&self) -> Report {
        Report::new(
            self.files
                .iter()
                .map(|file| {
                    FileOutcome::new(
                        file.relative_path.clone(),
                        file.status,
                        file.updated.is_some(),
                    )
                })
                .collect(),
        )
    }

    /// Atomically applies every planned edit after checking for stale inputs.
    pub fn apply(&self) -> Result<Report> {
        for file in &self.files {
            let (Some(original), Some(updated)) = (&file.original, &file.updated) else {
                continue;
            };
            write_atomic(&file.absolute_path, original, updated)?;
        }
        Ok(self.report())
    }
}

/// The analysis and optional replacement planned for one file.
pub struct PlannedFile {
    absolute_path: PathBuf,
    relative_path: PathBuf,
    status: Status,
    original: Option<Vec<u8>>,
    updated: Option<Vec<u8>>,
    file_attrs: Option<FileAttrs>,
}

impl PlannedFile {
    fn unsupported(absolute_path: PathBuf, relative_path: PathBuf) -> Self {
        Self {
            absolute_path,
            relative_path,
            status: Status::Unsupported,
            original: None,
            updated: None,
            file_attrs: None,
        }
    }

    /// Returns the canonical source path.
    pub fn absolute_path(&self) -> &Path {
        &self.absolute_path
    }

    /// Returns the source path relative to `files.root`.
    pub fn path(&self) -> &Path {
        &self.relative_path
    }

    /// Returns the file's analysis status.
    pub fn status(&self) -> Status {
        self.status
    }

    /// Returns whether this plan would modify the file.
    pub fn changed(&self) -> bool {
        self.updated.is_some()
    }

    /// Returns the original UTF-8 source when the file is supported text.
    pub fn original(&self) -> Option<&str> {
        self.original
            .as_deref()
            .and_then(|input| std::str::from_utf8(input).ok())
    }

    /// Returns the complete replacement source when an edit is planned.
    pub fn updated(&self) -> Option<&str> {
        self.updated
            .as_deref()
            .and_then(|input| std::str::from_utf8(input).ok())
    }

    /// Returns values exposed to the header template for this file.
    pub fn file_attrs(&self) -> Option<&FileAttrs> {
        self.file_attrs.as_ref()
    }
}
