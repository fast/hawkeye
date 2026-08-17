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
use std::path::PathBuf;

use crate::Error;
use crate::ErrorKind;
use crate::ResolvedConfig;
use crate::analyze::analyze;
use crate::attrs::FileAttrsResolver;
use crate::discovery::discover;
use crate::git::GitRepo;
use crate::report::FileOutcome;
use crate::report::Mode;
use crate::report::Report;
use crate::report::Status;
use crate::writer::validate_source;
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

    /// Discovers and analyzes files without modifying the filesystem.
    pub fn plan(&self, mode: Mode) -> Result<Plan, Error> {
        let git = self.config.git;
        let repo = GitRepo::discover(&self.config.root, git.ignore.combine(git.file_attrs))?;
        let paths = discover(&self.config, repo.as_ref())?;
        let attrs = FileAttrsResolver::new(&paths, git.file_attrs, repo.as_ref())?;
        let mut files = Vec::with_capacity(paths.len());

        for path in paths {
            let relative = path
                .strip_prefix(&self.config.root)
                .expect("discovery only returns paths inside files.root")
                .to_path_buf();
            if self.config.rule_for(&relative).is_none() {
                files.push(PlannedFile::unsupported(path, relative));
                continue;
            }

            let original = fs::read(&path).map_err(|source| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", path.display()),
                )
                .with_source(source)
            })?;
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
            let original = updated.as_ref().map(|_| original);
            files.push(PlannedFile {
                absolute_path: path,
                relative_path: relative,
                status: analysis.status,
                original,
                updated,
            });
        }

        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(Plan { files })
    }
}

/// A complete, deterministic operation plan produced before any file is written.
pub struct Plan {
    files: Vec<PlannedFile>,
}

impl Plan {
    /// Builds the serializable report for this plan.
    pub fn report(&self) -> Report {
        Report {
            files: self
                .files
                .iter()
                .map(|file| FileOutcome {
                    path: file.relative_path.clone(),
                    status: file.status,
                    changed: file.updated.is_some(),
                })
                .collect(),
        }
    }

    /// Atomically applies every planned edit after checking for stale inputs.
    pub fn apply(&self) -> Result<(), Error> {
        for file in &self.files {
            let (Some(original), Some(_)) = (&file.original, &file.updated) else {
                continue;
            };
            validate_source(&file.absolute_path, original)?;
        }
        for file in &self.files {
            let (Some(original), Some(updated)) = (&file.original, &file.updated) else {
                continue;
            };
            write_atomic(&file.absolute_path, original, updated)?;
        }
        Ok(())
    }
}

/// The analysis and optional replacement planned for one file.
struct PlannedFile {
    absolute_path: PathBuf,
    relative_path: PathBuf,
    status: Status,
    original: Option<Vec<u8>>,
    updated: Option<Vec<u8>>,
}

impl PlannedFile {
    fn unsupported(absolute_path: PathBuf, relative_path: PathBuf) -> Self {
        Self {
            absolute_path,
            relative_path,
            status: Status::Unsupported,
            original: None,
            updated: None,
        }
    }
}
