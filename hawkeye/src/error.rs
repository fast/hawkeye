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
use std::ops::Range;
use std::path::PathBuf;

use crate::config::ConfigError;

/// An error returned by HawkEye's library API.
#[derive(Debug)]
pub enum Error {
    /// The configuration could not be parsed or locally validated.
    Config(ConfigError),

    /// A configuration value could only be rejected while resolving resources.
    InvalidConfig(String),

    /// A template could not be compiled or rendered.
    Template(minijinja::Error),

    /// File discovery failed.
    Discovery(ignore::Error),

    /// A concrete filesystem operation failed.
    Io {
        /// The operation being attempted.
        operation: &'static str,
        /// The path involved in the failed operation.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// Git was required or started, but the operation failed.
    Git(String),

    /// A source file changed after its edit was planned.
    StaleFile(PathBuf),

    /// An edit does not point at a valid UTF-8 byte range in its original input.
    InvalidEdit {
        /// The invalid byte range.
        range: Range<usize>,
        /// The original input length.
        input_len: usize,
    },

    /// HawkEye refuses to replace a symbolic link.
    Symlink(PathBuf),

    /// HawkEye refuses to replace one name of a multiply linked file.
    HardLink(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::InvalidConfig(message) => {
                write!(formatter, "invalid licenserc.toml: {message}")
            }
            Self::Template(error) => write!(formatter, "cannot render header template: {error}"),
            Self::Discovery(error) => write!(formatter, "cannot discover files: {error}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "cannot {operation} {}: {source}", path.display()),
            Self::Git(message) => write!(formatter, "Git integration is unavailable: {message}"),
            Self::StaleFile(path) => {
                write!(
                    formatter,
                    "{} changed after it was analyzed",
                    path.display()
                )
            }
            Self::InvalidEdit { range, input_len } => write!(
                formatter,
                "invalid edit range {range:?} for an input of {input_len} bytes"
            ),
            Self::Symlink(path) => {
                write!(
                    formatter,
                    "refusing to replace symbolic link {}",
                    path.display()
                )
            }
            Self::HardLink(path) => write!(
                formatter,
                "refusing to replace hard-linked file {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::Template(error) => Some(error),
            Self::Discovery(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ConfigError> for Error {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<minijinja::Error> for Error {
    fn from(error: minijinja::Error) -> Self {
        Self::Template(error)
    }
}

impl From<ignore::Error> for Error {
    fn from(error: ignore::Error) -> Self {
        Self::Discovery(error)
    }
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
