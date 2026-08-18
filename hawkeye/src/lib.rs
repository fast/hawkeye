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

//! Check, format, and remove source-file license headers.
//!
//! Load a [`Config`], build an [`Engine`], and run the desired operation:
//!
//! ```no_run
//! use hawkeye::Config;
//! use hawkeye::Engine;
//!
//! # fn main() -> Result<(), hawkeye::Error> {
//! let config = Config::load("licenserc.toml")?;
//! let report = Engine::new(config)?.check()?;
//! println!("checked {} files", report.files.len());
//! # Ok(())
//! # }
//! ```
//!
//! The default `application` feature builds the `hawkeye` executable. Library-only users can
//! disable default features to omit its command-specific dependencies.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

mod builtin;
mod config;
mod engine;
mod error;
mod report;
mod template;

pub use self::config::Config;
pub use self::config::FeatureMode;
pub use self::config::FilesConfig;
pub use self::config::GitConfig;
pub use self::config::HeaderConfig;
pub use self::config::RuleConfig;
pub use self::config::StyleConfig;
pub use self::engine::Edits;
pub use self::engine::Engine;
pub use self::error::Error;
pub use self::error::ErrorKind;
pub use self::report::FileOutcome;
pub use self::report::FileReport;
pub use self::report::Report;
