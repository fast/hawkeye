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

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::time::Instant;

use crate::Error;
use crate::Result;
use crate::config::FeatureMode;

pub(crate) struct GitRepo {
    root: PathBuf,
}

impl GitRepo {
    pub(crate) fn discover(root: &Path, mode: FeatureMode) -> Result<Option<Self>> {
        if mode == FeatureMode::Disable {
            return Ok(None);
        }

        let started = Instant::now();
        let output = match Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["rev-parse", "--show-toplevel"])
            .output()
        {
            Ok(output) => output,
            Err(error) if mode == FeatureMode::Auto => {
                log::debug!(
                    "Git repository discovery is unavailable for {}: {error}",
                    root.display()
                );
                return Ok(None);
            }
            Err(error) => return Err(Error::Git(format!("cannot start Git: {error}"))),
        };

        if !output.status.success() {
            if mode == FeatureMode::Auto {
                log::debug!(
                    "Git repository discovery found no worktree for {}: {}",
                    root.display(),
                    stderr(&output)
                );
                return Ok(None);
            }
            return Err(Error::Git(stderr(&output)));
        }

        let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if path.is_empty() {
            return Err(Error::Git(
                "Git returned an empty repository root".to_owned(),
            ));
        }
        let root = PathBuf::from(path)
            .canonicalize()
            .map_err(|error| Error::Git(format!("cannot resolve repository root: {error}")))?;
        log::debug!(
            "discovered Git repository {} in {:?}",
            root.display(),
            started.elapsed()
        );
        Ok(Some(Self { root }))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn list_files(&self, scan_root: &Path) -> Result<Vec<PathBuf>> {
        let relative_root = scan_root.strip_prefix(&self.root).map_err(|_| {
            Error::Git(format!(
                "files.root {} is outside repository {}",
                scan_root.display(),
                self.root.display()
            ))
        })?;
        let pathspec = if relative_root.as_os_str().is_empty() {
            OsString::from(".")
        } else {
            relative_root.as_os_str().to_owned()
        };
        let output = self.output([
            OsString::from("ls-files"),
            OsString::from("--cached"),
            OsString::from("--others"),
            OsString::from("--exclude-standard"),
            OsString::from("-z"),
            OsString::from("--"),
            pathspec,
        ])?;

        let mut files = Vec::new();
        for record in output.stdout.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let path = self.root.join(path_from_git_bytes(record));
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(Error::io("read metadata for", &path, source)),
            };
            if metadata.file_type().is_file() && path.starts_with(scan_root) {
                files.push(path);
            }
        }
        Ok(files)
    }

    pub(crate) fn has_head(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["-C"])
            .arg(&self.root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .map_err(|error| Error::Git(format!("cannot inspect Git HEAD: {error}")))?;
        Ok(output.status.success())
    }

    pub(crate) fn optional_config(&self, key: &str) -> Result<Option<String>> {
        let output = Command::new("git")
            .args(["-C"])
            .arg(&self.root)
            .args(["config", "--get", key])
            .output()
            .map_err(|error| Error::Git(format!("cannot read {key}: {error}")))?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok((!value.is_empty()).then_some(value))
    }

    pub(crate) fn output<I, S>(&self, arguments: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        let started = Instant::now();
        let output = Command::new("git")
            .args(["-C"])
            .arg(&self.root)
            .args(&arguments)
            .output()
            .map_err(|error| Error::Git(format!("cannot start Git: {error}")))?;
        log::debug!("Git {:?} completed in {:?}", arguments, started.elapsed());
        if output.status.success() {
            Ok(output)
        } else {
            Err(Error::Git(stderr(&output)))
        }
    }
}

#[cfg(unix)]
pub(crate) fn git_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
pub(crate) fn git_path(path: &Path) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}

fn stderr(output: &Output) -> String {
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if message.is_empty() {
        format!("Git exited with {}", output.status)
    } else {
        message
    }
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
