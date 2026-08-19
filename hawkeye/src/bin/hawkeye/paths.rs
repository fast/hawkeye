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
use std::fs;
use std::io;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use clap::Args;
use exn::Result;

use crate::Error;

#[derive(Debug, Args)]
pub struct PathOptions {
    /// Files and directories to process.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Read one UTF-8 path per line from FILE; use `-` for stdin.
    #[arg(long, value_name = "FILE")]
    files_from: Option<PathBuf>,
}

impl PathOptions {
    pub fn into_paths(mut self) -> Result<Option<Vec<PathBuf>>, Error> {
        if self.paths.is_empty() && self.files_from.is_none() {
            return Ok(None);
        }

        if let Some(path) = self.files_from {
            self.paths.extend(read_path_list(&path)?);
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

fn read_path_list(path: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut content = String::new();
    if path == Path::new("-") {
        io::stdin()
            .read_to_string(&mut content)
            .map_err(|err| Error::new(format!("cannot read paths from stdin: {err}")))?;
    } else {
        content = fs::read_to_string(path).map_err(|err| {
            Error::new(format!("cannot read paths from {}: {err}", path.display()))
        })?;
    }

    Ok(content
        .lines()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect())
}
