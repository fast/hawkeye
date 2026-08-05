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

use std::ops::Range;
use std::path::PathBuf;

use thiserror::Error;

/// An error returned by the HawkEye library.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The TOML document is syntactically invalid or does not match the v7 schema.
    #[error("cannot parse HawkEye configuration: {0}")]
    ConfigParse(#[from] toml::de::Error),

    /// The configuration parses but violates a semantic invariant.
    #[error("invalid HawkEye configuration: {0}")]
    InvalidConfig(String),

    /// Header template parsing or rendering failed.
    #[error("cannot render license header template: {0}")]
    Template(#[from] minijinja::Error),

    /// File discovery could not traverse part of the configured root.
    #[error("cannot discover files: {0}")]
    Discovery(#[from] ignore::Error),

    /// A filesystem operation failed for a concrete path.
    #[error("cannot {operation} {}: {source}", path.display())]
    Io {
        /// The operation being attempted.
        operation: &'static str,
        /// The affected path.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// Rewriting a symbolic link is intentionally unsupported.
    #[error("refusing to replace symbolic link {}", .0.display())]
    Symlink(PathBuf),

    /// An edit range is not valid for the supplied input.
    #[error("edit range {range:?} is invalid for an input of {input_len} bytes")]
    InvalidEdit {
        /// The invalid byte range.
        range: Range<usize>,
        /// The current input length.
        input_len: usize,
    },

    /// The input changed after its edit was planned.
    #[error("the input changed after the edit was planned")]
    StaleEdit,

    /// A source file changed after the repository plan was created.
    #[error("{} changed after it was analyzed", .0.display())]
    StaleFile(PathBuf),
}

/// A result returned by the HawkEye library.
pub type Result<T> = std::result::Result<T, Error>;
