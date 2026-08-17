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

/// A stable, actionable category of failures returned by HawkEye.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The configuration or header template must be corrected.
    ConfigInvalid,
    /// The requested operation cannot be performed for the current input or environment.
    Unsupported,
    /// A file changed after the operation was planned; callers may create a new plan and retry.
    StalePlan,
    /// The operation failed in a way that callers cannot reliably recover from.
    Unexpected,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConfigInvalid => "ConfigInvalid",
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
    source: Option<String>,
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

    pub(crate) fn with_source(mut self, source: impl fmt::Display) -> Self {
        self.source = Some(source.to_string());
        self
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
        write!(formatter, "{}", self.kind)?;
        if !self.message.is_empty() {
            write!(formatter, " => {}", self.message)?;
        }
        if let Some(source) = &self.source {
            write!(formatter, ", source: {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}
