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

use crate::config::ConfigError;

/// An error returned by HawkEye's library API.
#[derive(Debug, Error)]
pub enum Error {
    /// The configuration could not be parsed or locally validated.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// A configuration value could only be rejected while resolving resources.
    #[error("invalid licenserc.toml: {0}")]
    InvalidConfig(String),

    /// A template could not be compiled or rendered.
    #[error("cannot render header template: {0}")]
    Template(#[from] minijinja::Error),

    /// File discovery failed.
    #[error("cannot discover files: {0}")]
    Discovery(#[from] ignore::Error),

    /// A concrete filesystem operation failed.
    #[error("cannot {operation} {}: {source}", path.display())]
    Io {
        /// The operation being attempted.
        operation: &'static str,
        /// The path involved in the failed operation.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Git was required or started, but the operation failed.
    #[error("Git integration is unavailable: {0}")]
    Git(String),

    /// A positional file or directory cannot be processed under `files.root`.
    #[error("invalid explicit target: {0}")]
    InvalidTarget(String),

    /// A source file changed after its edit was planned.
    #[error("{} changed after it was analyzed", .0.display())]
    StaleFile(PathBuf),

    /// An edit does not point at a valid UTF-8 byte range in its original input.
    #[error("invalid edit range {range:?} for an input of {input_len} bytes")]
    InvalidEdit {
        /// The invalid byte range.
        range: Range<usize>,
        /// The original input length.
        input_len: usize,
    },

    /// HawkEye refuses to replace a symbolic link.
    #[error("refusing to replace symbolic link {}", .0.display())]
    Symlink(PathBuf),

    /// HawkEye refuses to replace one name of a multiply linked file.
    #[error("refusing to replace hard-linked file {}", .0.display())]
    HardLink(PathBuf),
}

impl Error {
    pub(crate) fn io(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}

/// A result returned by HawkEye's library API.
pub type Result<T> = std::result::Result<T, Error>;
