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

use crate::Edits;
use crate::Error;
use crate::ErrorKind;
use crate::Report;
use crate::engine::FileEdit;

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
    /// Each file is read once before writing. Contents that already match the intended result
    /// are skipped; otherwise they must still match the original contents. Writes preserve file
    /// symlinks and hard links, so identical edits through multiple aliases do not accumulate.
    ///
    /// Application stops at the first error without rolling back earlier writes. Callers must
    /// prevent concurrent writes while applying edits.
    ///
    /// # Errors
    ///
    /// Returns an error if a source matches neither the original nor the intended contents, or
    /// cannot be read or written.
    pub fn apply(self) -> Result<Report, Error> {
        for FileEdit {
            path,
            original,
            replacement,
        } in self.files
        {
            let mut output = original.clone();
            output.replace_range(replacement.range, &replacement.text);
            let current = fs::read(&path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", path.display()),
                )
                .with_source(err)
            })?;
            if current == output.as_bytes() {
                continue;
            }
            if current != original.as_bytes() {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "{} changed since edits were prepared; prepare edits again",
                        path.display()
                    ),
                ));
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
