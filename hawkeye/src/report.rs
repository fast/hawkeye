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

/// The outcome of an operation for one selected file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FileOutcome {
    /// The file already satisfies the requested operation.
    #[serde(rename = "clean")]
    Clean,
    /// The operation requires adding a canonical header.
    #[serde(rename = "add")]
    Add,
    /// The operation requires replacing a recognized header.
    #[serde(rename = "replace")]
    Replace,
    /// The operation requires removing a recognized header.
    #[serde(rename = "remove")]
    Remove,
    /// A header-like comment cannot be edited safely.
    #[serde(rename = "conflict")]
    Conflict,
    /// The file has no rule or is not supported UTF-8 text.
    #[serde(rename = "unsupported")]
    Unsupported,
}

/// The outcome for one selected file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileReport {
    /// The path relative to `files.root`.
    pub path: PathBuf,
    /// The operation outcome.
    pub outcome: FileOutcome,
}

/// The outcomes of an [`Engine`](crate::Engine) operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// One entry for each selected file.
    pub files: Vec<FileReport>,
}
