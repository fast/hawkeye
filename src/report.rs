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

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;
use serde::Serializer;

/// An operation supported by the analysis engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mode {
    /// Report files whose headers are not compliant without changing them.
    Check,
    /// Add or replace headers when a safe edit can be proven.
    Format,
    /// Remove headers when their exact source range can be proven.
    Remove,
}

/// The result of analyzing one selected file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Status {
    /// The preferred header is already present with the expected content.
    Clean,
    /// No license header candidate was found.
    Missing,
    /// A header has a proven range and can be safely rewritten.
    Replaceable,
    /// A license-like prefix exists, but no safe edit range can be proven.
    Conflict,
    /// The file has no applicable rule or cannot be analyzed as supported text.
    Unsupported,
}

impl Status {
    /// Returns whether `check` treats this status as a policy violation.
    pub fn is_violation(self) -> bool {
        matches!(self, Self::Missing | Self::Replaceable | Self::Conflict)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Clean => "clean",
            Self::Missing => "missing",
            Self::Replaceable => "replaceable",
            Self::Conflict => "conflict",
            Self::Unsupported => "unsupported",
        })
    }
}

/// The deterministic analysis outcome for one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileOutcome {
    #[serde(serialize_with = "serialize_path")]
    path: PathBuf,
    status: Status,
}

impl FileOutcome {
    /// Creates an outcome for `path`.
    pub fn new(path: impl Into<PathBuf>, status: Status) -> Self {
        Self {
            path: path.into(),
            status,
        }
    }

    /// Returns the path relative to the configured root when possible.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Returns the file status.
    pub fn status(&self) -> Status {
        self.status
    }
}

fn serialize_path<SerializerType>(
    path: &Path,
    serializer: SerializerType,
) -> Result<SerializerType::Ok, SerializerType::Error>
where
    SerializerType: Serializer,
{
    serializer.serialize_str(&path.to_string_lossy())
}

/// A stable, path-sorted report returned by the library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    files: Vec<FileOutcome>,
}

impl Report {
    /// Creates a report and sorts its outcomes by path for deterministic presentation.
    pub fn new(mut files: Vec<FileOutcome>) -> Self {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Self { files }
    }

    /// Returns every selected file outcome in deterministic path order.
    pub fn files(&self) -> &[FileOutcome] {
        &self.files
    }

    /// Returns whether the report contains a missing, replaceable, or conflicting header.
    pub fn has_violations(&self) -> bool {
        self.files.iter().any(|file| file.status.is_violation())
    }

    /// Counts files in one status.
    pub fn count(&self, status: Status) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == status)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_order_is_deterministic() {
        let report = Report::new(vec![
            FileOutcome::new("src/z.rs", Status::Clean),
            FileOutcome::new("src/a.rs", Status::Missing),
        ]);

        let paths = report
            .files()
            .iter()
            .map(FileOutcome::path)
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                std::path::Path::new("src/a.rs"),
                std::path::Path::new("src/z.rs")
            ]
        );
    }
}
