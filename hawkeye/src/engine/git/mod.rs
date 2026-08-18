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

mod command;
mod history;

use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use self::command::git_command;
use self::command::stderr;
pub use self::history::GitFileHistory;
use crate::Error;
use crate::ErrorKind;

pub struct GitRepo {
    root: PathBuf,
}

impl GitRepo {
    pub fn discover(root: &Path) -> Result<Self, Error> {
        let started = Instant::now();
        let output = match git_command(root)
            .args(["rev-parse", "--show-toplevel"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("Git cannot be started for {}", root.display()),
                )
                .with_source(err));
            }
        };

        if !output.status.success() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("{} is not a usable Git worktree", root.display()),
            )
            .with_source(stderr(&output)));
        }

        let path = output
            .stdout
            .strip_suffix(b"\n")
            .unwrap_or(output.stdout.as_slice());
        if path.is_empty() {
            return Err(Error::new(
                ErrorKind::Unexpected,
                "Git returned an empty repository root",
            ));
        }
        let root = path_from_git_bytes(path).canonicalize().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot resolve repository root").with_source(err)
        })?;
        log::debug!(
            "discovered Git repository {} in {:?}",
            root.display(),
            started.elapsed()
        );
        Ok(Self { root })
    }

    pub fn list_files(&self, scan_root: &Path) -> Result<Vec<PathBuf>, Error> {
        let relative_root = scan_root.strip_prefix(&self.root).map_err(|_| {
            Error::new(
                ErrorKind::Unexpected,
                format!(
                    "files.root {} is outside repository {}",
                    scan_root.display(),
                    self.root.display()
                ),
            )
        })?;
        let pathspec = if relative_root.as_os_str().is_empty() {
            OsString::from(".")
        } else {
            relative_root.as_os_str().to_owned()
        };
        let output = self.output(
            "cannot list Git worktree files",
            [
                OsString::from("ls-files"),
                OsString::from("--cached"),
                OsString::from("--others"),
                OsString::from("--exclude-standard"),
                OsString::from("-z"),
                OsString::from("--"),
                pathspec,
            ],
        )?;

        let mut files = Vec::new();
        for record in output.stdout.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let path = self.root.join(path_from_git_bytes(record));
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(Error::new(
                        ErrorKind::Unexpected,
                        format!("cannot read metadata for {}", path.display()),
                    )
                    .with_source(err));
                }
            };
            let file_type = metadata.file_type();
            if (file_type.is_file() || (file_type.is_symlink() && path.is_file()))
                && path.starts_with(scan_root)
            {
                files.push(path);
            }
        }
        Ok(files)
    }

    pub fn is_shallow(&self) -> Result<bool, Error> {
        let output = self.output(
            "cannot inspect whether the Git repository is shallow",
            ["rev-parse", "--is-shallow-repository"],
        )?;
        match String::from_utf8_lossy(&output.stdout).trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(Error::new(
                ErrorKind::Unexpected,
                format!("Git returned an invalid shallow-repository value: {value:?}"),
            )),
        }
    }
}

#[cfg(unix)]
fn git_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn git_path(path: &Path) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}
