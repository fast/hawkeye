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
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Instant;

use crate::Error;
use crate::ErrorKind;
use crate::config::FeatureMode;

pub(crate) struct GitRepo {
    root: PathBuf,
}

impl GitRepo {
    pub(crate) fn discover(root: &Path, mode: FeatureMode) -> Result<Option<Self>, Error> {
        if mode == FeatureMode::Disable {
            return Ok(None);
        }

        let started = Instant::now();
        let output = match git_command(root)
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
            Err(source) => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!(
                        "Git is required for {} but cannot be started",
                        root.display()
                    ),
                )
                .with_source(source));
            }
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
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "Git is required but {} is not a usable Git worktree",
                    root.display()
                ),
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
        let root = path_from_git_bytes(path).canonicalize().map_err(|source| {
            Error::new(ErrorKind::Unexpected, "cannot resolve repository root").with_source(source)
        })?;
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

    pub(crate) fn list_files(&self, scan_root: &Path) -> Result<Vec<PathBuf>, Error> {
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
                Err(source) => {
                    return Err(Error::new(
                        ErrorKind::Unexpected,
                        format!("cannot read metadata for {}", path.display()),
                    )
                    .with_source(source));
                }
            };
            if metadata.file_type().is_file() && path.starts_with(scan_root) {
                files.push(path);
            }
        }
        Ok(files)
    }

    pub(crate) fn has_head(&self) -> Result<bool, Error> {
        let output = self.output_unchecked(["rev-parse", "--verify", "--quiet", "HEAD"])?;
        if output.status.success() {
            Ok(true)
        } else if output.status.code() == Some(1) {
            Ok(false)
        } else {
            Err(Error::new(ErrorKind::Unexpected, "cannot inspect Git HEAD")
                .with_source(stderr(&output)))
        }
    }

    pub(crate) fn is_shallow(&self) -> Result<bool, Error> {
        let output = self.output(["rev-parse", "--is-shallow-repository"])?;
        match String::from_utf8_lossy(&output.stdout).trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(Error::new(
                ErrorKind::Unexpected,
                format!("Git returned an invalid shallow-repository value: {value:?}"),
            )),
        }
    }

    pub(crate) fn optional_config(&self, key: &str) -> Result<Option<String>, Error> {
        let output = self.output_unchecked(["config", "--get", key])?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(Error::new(
                ErrorKind::Unexpected,
                format!("cannot read Git configuration key {key}"),
            )
            .with_source(stderr(&output)));
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok((!value.is_empty()).then_some(value))
    }

    pub(crate) fn output<I, S>(&self, arguments: I) -> Result<Output, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output_unchecked(arguments)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(Error::new(ErrorKind::Unexpected, stderr(&output)))
        }
    }

    fn output_unchecked<I, S>(&self, arguments: I) -> Result<Output, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        let started = Instant::now();
        let output = git_command(&self.root)
            .args(&arguments)
            .output()
            .map_err(|source| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot execute Git {arguments:?}"),
                )
                .with_source(source)
            })?;
        log::debug!("Git {:?} completed in {:?}", arguments, started.elapsed());
        Ok(output)
    }

    pub(crate) fn read_stdout<I, S, Value>(
        &self,
        arguments: I,
        input: Option<&[u8]>,
        read: impl FnOnce(&mut dyn BufRead) -> Result<Value, Error>,
    ) -> Result<Value, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        let mut stderr = tempfile::tempfile().map_err(|source| {
            Error::new(ErrorKind::Unexpected, "cannot create Git stderr buffer").with_source(source)
        })?;
        let stderr_writer = stderr.try_clone().map_err(|source| {
            Error::new(ErrorKind::Unexpected, "cannot clone Git stderr buffer").with_source(source)
        })?;
        let stdin = if let Some(input) = input {
            let mut file = tempfile::tempfile().map_err(|source| {
                Error::new(ErrorKind::Unexpected, "cannot create Git stdin buffer")
                    .with_source(source)
            })?;
            file.write_all(input).map_err(|source| {
                Error::new(ErrorKind::Unexpected, "cannot write Git stdin buffer")
                    .with_source(source)
            })?;
            file.seek(SeekFrom::Start(0)).map_err(|source| {
                Error::new(ErrorKind::Unexpected, "cannot rewind Git stdin buffer")
                    .with_source(source)
            })?;
            Stdio::from(file)
        } else {
            Stdio::null()
        };
        let started = Instant::now();
        let mut child = git_command(&self.root)
            .args(&arguments)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr_writer))
            .spawn()
            .map_err(|source| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot execute Git {arguments:?}"),
                )
                .with_source(source)
            })?;
        let stdout = child
            .stdout
            .take()
            .expect("Git stdout was configured as a pipe");
        let parsed = read(&mut BufReader::new(stdout));
        if parsed.is_err() {
            let _ = child.kill();
        }
        let status = child.wait().map_err(|source| {
            Error::new(ErrorKind::Unexpected, "cannot wait for Git").with_source(source)
        })?;
        log::debug!("Git {:?} completed in {:?}", arguments, started.elapsed());
        let value = parsed?;
        if status.success() {
            return Ok(value);
        }

        stderr.seek(SeekFrom::Start(0)).map_err(|source| {
            Error::new(ErrorKind::Unexpected, "cannot rewind Git stderr").with_source(source)
        })?;
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map_err(|source| {
            Error::new(ErrorKind::Unexpected, "cannot read Git stderr").with_source(source)
        })?;
        let message = String::from_utf8_lossy(&bytes).trim().to_owned();
        if message.is_empty() {
            Err(Error::new(
                ErrorKind::Unexpected,
                format!("Git exited with {status}"),
            ))
        } else {
            Err(Error::new(ErrorKind::Unexpected, message))
        }
    }
}

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .env("GIT_LITERAL_PATHSPECS", "1");
    command
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
