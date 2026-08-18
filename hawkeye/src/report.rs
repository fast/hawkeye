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

use std::path::PathBuf;

use serde::Serialize;

/// The planned outcome for one selected file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// No change is needed.
    Clean,
    /// A canonical header should be added.
    Add,
    /// A recognized header should be replaced.
    Replace,
    /// A recognized header should be removed.
    Remove,
    /// The file looks licensed but no safe edit range is known.
    Conflict,
    /// The file has no rule or is not supported UTF-8 text.
    Unsupported,
}

/// The outcome for one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileOutcome {
    /// The path relative to `files.root`.
    pub path: PathBuf,
    /// The planned outcome.
    pub outcome: Outcome,
}

/// A report produced from a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// All selected file outcomes.
    pub files: Vec<FileOutcome>,
}
