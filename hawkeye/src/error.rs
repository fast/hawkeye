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

/// A stable, actionable category of failures returned by HawkEye.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The configuration or header template must be corrected.
    ConfigInvalid,
    /// A filesystem operation failed.
    Io,
    /// A required Git operation or repository capability is unavailable.
    GitUnavailable,
    /// HawkEye deliberately refuses an operation that it cannot perform safely.
    Unsupported,
    /// A file changed after the operation was planned; callers may create a new plan and retry.
    StalePlan,
    /// An internal invariant failed and the operation cannot be recovered by the caller.
    Unexpected,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConfigInvalid => "ConfigInvalid",
            Self::Io => "Io",
            Self::GitUnavailable => "GitUnavailable",
            Self::Unsupported => "Unsupported",
            Self::StalePlan => "StalePlan",
            Self::Unexpected => "Unexpected",
        })
    }
}

/// An error returned by HawkEye's library API.
pub struct Error {
    kind: ErrorKind,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    /// Returns the stable category that callers can act on.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn with_source(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub(crate) fn config(message: impl fmt::Display) -> Self {
        Self::new(
            ErrorKind::ConfigInvalid,
            format!("invalid licenserc.toml: {message}"),
        )
    }

    pub(crate) fn config_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::new(ErrorKind::ConfigInvalid, message).with_source(source)
    }

    pub(crate) fn git(message: impl fmt::Display) -> Self {
        Self::new(
            ErrorKind::GitUnavailable,
            format!("Git integration is unavailable: {message}"),
        )
    }

    pub(crate) fn io(operation: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::new(
            ErrorKind::Io,
            format!("cannot {operation} {}", path.display()),
        )
        .with_source(source)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if formatter.alternate() {
            return formatter
                .debug_struct("Error")
                .field("kind", &self.kind)
                .field("message", &self.message)
                .field("source", &self.source)
                .finish();
        }

        write!(formatter, "{} => {}", self.kind, self.message)?;
        if let Some(source) = &self.source {
            write!(formatter, "\n\nSource:\n   {source}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

impl From<minijinja::Error> for Error {
    fn from(source: minijinja::Error) -> Self {
        Self::new(ErrorKind::ConfigInvalid, "cannot render header template").with_source(source)
    }
}

impl From<ignore::Error> for Error {
    fn from(source: ignore::Error) -> Self {
        let kind = if source.is_io() {
            ErrorKind::Io
        } else {
            ErrorKind::Unexpected
        };
        Self::new(kind, "cannot discover files").with_source(source)
    }
}

/// A result returned by HawkEye's library API.
pub type Result<T> = std::result::Result<T, Error>;
