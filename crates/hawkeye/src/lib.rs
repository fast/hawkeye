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
pub mod config;
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

pub use attrs::FileAttrs;
pub use engine::Engine;
pub use engine::Plan;
pub use engine::PlannedFile;
pub use error::Error;
pub use error::Result;
pub use report::FileOutcome;
pub use report::Mode;
pub use report::Report;
pub use report::Status;
pub use resolved::ResolvedConfig;
