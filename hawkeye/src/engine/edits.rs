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

use super::Replacement;
use crate::Error;
use crate::ErrorKind;
use crate::Report;

/// Pending file edits prepared by an [`Engine`](crate::Engine).
#[must_use = "edits have no effect until they are applied"]
pub struct Edits {
    pub(super) report: Report,
    pub(super) files: Vec<FileEdit>,
}

impl Edits {
    /// Inspects the planned outcomes without applying or discarding the edits.
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// Discards the pending edits and returns their report.
    pub fn into_report(self) -> Report {
        self.report
    }

    /// Applies the pending edits and returns their report.
    ///
    /// All files to be edited are checked against their original contents before writing begins.
    /// Writes preserve file symlinks and hard links; identical edits through multiple aliases do
    /// not accumulate. Incompatible edits through aliases return an error.
    ///
    /// This is not a filesystem transaction: a later I/O error or incompatible alias can leave
    /// earlier writes applied. Callers must prevent concurrent writes while applying edits.
    ///
    /// # Errors
    ///
    /// Returns an error if a source has changed since preparation or cannot be read or written.
    pub fn apply(self) -> Result<Report, Error> {
        for file in &self.files {
            if read(&file.path)? != file.original.as_bytes() {
                return Err(source_changed(&file.path));
            }
        }

        for FileEdit {
            path,
            original,
            replacement,
        } in self.files
        {
            let mut output = original.clone();
            output.replace_range(replacement.range, &replacement.text);
            let current = read(&path)?;
            if current == output.as_bytes() {
                continue;
            }
            if current != original.as_bytes() {
                return Err(source_changed(&path));
            }
            fs::write(&path, output).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot write {}", path.display()),
                )
                .with_source(err)
            })?;
        }
        Ok(self.report)
    }
}

pub(super) struct FileEdit {
    pub path: PathBuf,
    pub original: String,
    pub replacement: Replacement,
}

fn read(path: &Path) -> Result<Vec<u8>, Error> {
    fs::read(path).map_err(|err| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot read {}", path.display()),
        )
        .with_source(err)
    })
}

fn source_changed(path: &Path) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!(
            "{} changed since edits were prepared; prepare edits again",
            path.display()
        ),
    )
}
