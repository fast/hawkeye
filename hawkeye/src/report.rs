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

/// A HawkEye operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Report non-compliant files without planning writes.
    Check,
    /// Insert or normalize headers.
    Format,
    /// Remove structurally recognized headers.
    Remove,
}

/// The result of analyzing one selected file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    /// The file already contains the canonical header.
    #[serde(rename = "clean")]
    Clean,
    /// No recognized header was found.
    #[serde(rename = "missing")]
    Missing,
    /// A recognized header differs in content, style, or spacing.
    #[serde(rename = "replaceable")]
    Replaceable,
    /// The file looks licensed but no safe edit range is known.
    #[serde(rename = "conflict")]
    Conflict,
    /// The file has no rule or is not supported UTF-8 text.
    #[serde(rename = "unsupported")]
    Unsupported,
}

impl Status {
    /// Returns whether `check` treats this state as non-compliant.
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

/// The deterministic outcome for one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileOutcome {
    /// The path relative to `files.root`.
    #[serde(serialize_with = "serialize_path")]
    pub path: PathBuf,
    /// The analysis status.
    pub status: Status,
    /// Whether the requested operation planned a modification.
    pub changed: bool,
}

/// A path-sorted operation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// All selected file outcomes in path order.
    pub files: Vec<FileOutcome>,
}

impl Report {
    /// Returns the number of planned or applied changes.
    pub fn changed(&self) -> usize {
        self.files.iter().filter(|file| file.changed).count()
    }

    /// Returns whether check found a policy violation.
    pub fn has_violations(&self) -> bool {
        self.files.iter().any(|file| file.status.is_violation())
    }

    /// Counts outcomes with a specific status.
    pub fn count(&self, status: Status) -> usize {
        self.files
            .iter()
            .filter(|file| file.status == status)
            .count()
    }
}

fn serialize_path<SerializerType>(
    path: &Path,
    serializer: SerializerType,
) -> Result<SerializerType::Ok, SerializerType::Error>
where
    SerializerType: Serializer,
{
    serializer.serialize_str(&path.to_string_lossy().replace('\\', "/"))
}
