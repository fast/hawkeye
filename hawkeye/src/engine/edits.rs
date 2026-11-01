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
    /// Callers must ensure that selected files remain unchanged from preparation until application
    /// completes. Edits do not detect concurrent changes.
    ///
    /// Files are read and written one at a time. Application stops at the first error without
    /// rolling back earlier writes.
    ///
    /// # Errors
    ///
    /// Returns an error if a source file cannot be read as UTF-8 text or written.
    pub fn apply(self) -> Result<Report, Error> {
        for FileEdit { path, replacement } in self.files {
            let mut input = fs::read_to_string(&path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", path.display()),
                )
                .with_source(err)
            })?;
            input.replace_range(replacement.range, &replacement.text);
            fs::write(&path, input).map_err(|err| {
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
