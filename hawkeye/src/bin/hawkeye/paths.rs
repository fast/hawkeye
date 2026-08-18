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
#[cfg(unix)]
use std::ffi::OsString;
use std::fs;
use std::io;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
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

    /// Read newline- or NUL-separated paths from FILE; use `-` for stdin.
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
    let mut content = Vec::new();
    if path == Path::new("-") {
        io::stdin()
            .read_to_end(&mut content)
            .map_err(|err| Error::new(format!("cannot read paths from stdin: {err}")))?;
    } else {
        content = fs::read(path).map_err(|err| {
            Error::new(format!("cannot read paths from {}: {err}", path.display()))
        })?;
    }

    // NUL cannot occur within a path, so its presence unambiguously selects NUL-delimited input.
    let nul_separated = content.contains(&b'\0');
    let mut paths = Vec::new();
    for mut value in content.split(|byte| {
        if nul_separated {
            *byte == b'\0'
        } else {
            *byte == b'\n'
        }
    }) {
        if !nul_separated && value.last() == Some(&b'\r') {
            value = &value[..value.len() - 1];
        }
        if !value.is_empty() {
            paths.push(path_from_bytes(value.to_vec())?);
        }
    }
    Ok(paths)
}

#[cfg(unix)]
fn path_from_bytes(value: Vec<u8>) -> Result<PathBuf, Error> {
    // Unix paths need not be UTF-8; preserve every record exactly as the producer emitted it.
    Ok(PathBuf::from(OsString::from_vec(value)))
}

#[cfg(not(unix))]
fn path_from_bytes(value: Vec<u8>) -> Result<PathBuf, Error> {
    let value = String::from_utf8(value)
        .map_err(|err| Error::new(format!("path list contains non-UTF-8 data: {err}")))?;
    Ok(value.into())
}
