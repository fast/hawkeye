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

use std::env;
use std::path::PathBuf;

use clap::Args;
use exn::Result;

use crate::Error;

#[derive(Debug, Args)]
pub struct PathOptions {
    /// Files and directories to process.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

impl PathOptions {
    pub fn into_paths(mut self) -> Result<Option<Vec<PathBuf>>, Error> {
        if self.paths.is_empty() {
            return Ok(None);
        }

        // CLI paths follow the current directory; Engine paths follow the configured file root.
        let current_dir = env::current_dir()
            .map_err(|err| Error::new(format!("cannot resolve the current directory: {err}")))?;
        for path in &mut self.paths {
            if path.is_relative() {
                *path = current_dir.join(&*path);
            }
        }
        Ok(Some(self.paths))
    }
}
