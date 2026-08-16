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

//! Library support for HawkEye.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

mod analyze;
mod attrs;
mod discovery;
mod edit;
mod engine;
mod error;
mod git;
mod report;
mod resolved;
mod style;
mod template;
mod writer;

pub mod config;
pub use self::attrs::FileAttrs;
pub use self::engine::Engine;
pub use self::engine::Plan;
pub use self::engine::PlannedFile;
pub use self::error::Error;
pub use self::error::Result;
pub use self::report::FileOutcome;
pub use self::report::Mode;
pub use self::report::Report;
pub use self::report::Status;
pub use self::resolved::ResolvedConfig;
