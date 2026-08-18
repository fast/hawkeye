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

mod history;

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

pub use self::history::FileHistory;
use crate::Error;
use crate::ErrorKind;

pub struct Repository {
    root: PathBuf,
}

impl Repository {
    pub fn discover(root: &Path) -> Result<Self, Error> {
        let started = Instant::now();
        let output = match command(root)
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
            .with_source(output_failure(&output)));
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
        let root = decode_path(path).canonicalize().map_err(|err| {
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
        let relative_root = self.relative_scan_root(scan_root)?;
        let pathspec = if relative_root.as_os_str().is_empty() {
            OsString::from(".")
        } else {
            relative_root.as_os_str().to_owned()
        };
        self.parse_stdout(
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
            None,
            |reader| {
                let mut files = Vec::new();
                let mut record = Vec::new();
                while read_record(reader, &mut record, "cannot read Git worktree files")? {
                    if record.is_empty() {
                        continue;
                    }
                    let path = self.root.join(decode_path(&record));
                    let metadata = match std::fs::symlink_metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(err) => {
                            return Err(Error::new(
                                ErrorKind::Unexpected,
                                format!("cannot read metadata for {}", path.display()),
                            )
                            .with_source(err));
                        }
                    };
                    let file_type = metadata.file_type();
                    if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
                        let relative = path.strip_prefix(scan_root).map_err(|_| {
                            Error::new(
                                ErrorKind::Unexpected,
                                format!(
                                    "Git returned path outside files.root {}: {}",
                                    scan_root.display(),
                                    path.display()
                                ),
                            )
                        })?;
                        files.push(relative.to_path_buf());
                    }
                }
                Ok(files)
            },
        )
    }

    pub fn is_shallow(&self) -> Result<bool, Error> {
        let output = self.run(
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

    fn relative_scan_root<'a>(&self, scan_root: &'a Path) -> Result<&'a Path, Error> {
        scan_root.strip_prefix(&self.root).map_err(|_| {
            Error::new(
                ErrorKind::Unexpected,
                format!(
                    "files.root {} is outside repository {}",
                    scan_root.display(),
                    self.root.display()
                ),
            )
        })
    }

    fn run<I, S>(&self, operation: &str, arguments: I) -> Result<Output, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run_unchecked(arguments)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(Error::new(ErrorKind::Unexpected, operation).with_source(output_failure(&output)))
        }
    }

    fn run_unchecked<I, S>(&self, arguments: I) -> Result<Output, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        let started = Instant::now();
        let output = command(&self.root)
            .args(&arguments)
            .output()
            .map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot execute Git {arguments:?}"),
                )
                .with_source(err)
            })?;
        log::debug!("Git {:?} completed in {:?}", arguments, started.elapsed());
        Ok(output)
    }

    fn parse_stdout<I, S, Value>(
        &self,
        operation: &str,
        arguments: I,
        stdin: Option<&[u8]>,
        parse: impl FnOnce(&mut dyn BufRead) -> Result<Value, Error>,
    ) -> Result<Value, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        // File-backed stdin and stderr let Git consume and produce both while stdout is parsed
        // synchronously, without another thread or the risk of a full pipe blocking the child.
        let mut stderr = tempfile::tempfile().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot create Git stderr buffer").with_source(err)
        })?;
        let stderr_writer = stderr.try_clone().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot clone Git stderr buffer").with_source(err)
        })?;
        let stdin = if let Some(input) = stdin {
            let mut file = tempfile::tempfile().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot create Git stdin buffer").with_source(err)
            })?;
            file.write_all(input).map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot write Git stdin buffer").with_source(err)
            })?;
            file.seek(SeekFrom::Start(0)).map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot rewind Git stdin buffer").with_source(err)
            })?;
            Stdio::from(file)
        } else {
            Stdio::null()
        };
        let started = Instant::now();
        let mut child = command(&self.root)
            .args(&arguments)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr_writer))
            .spawn()
            .map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot execute Git {arguments:?}"),
                )
                .with_source(err)
            })?;
        let stdout = child
            .stdout
            .take()
            .expect("Git stdout must be configured as a pipe");
        let parsed = parse(&mut BufReader::new(stdout));
        if parsed.is_err() {
            let _ = child.kill();
        }
        let status = child.wait().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot wait for Git").with_source(err)
        })?;
        log::debug!("Git {:?} completed in {:?}", arguments, started.elapsed());
        let value = parsed?;
        if status.success() {
            return Ok(value);
        }

        stderr.seek(SeekFrom::Start(0)).map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot rewind Git stderr").with_source(err)
        })?;
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot read Git stderr").with_source(err)
        })?;
        Err(Error::new(ErrorKind::Unexpected, operation)
            .with_source(failure_message(&status, &bytes)))
    }
}

fn command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn output_failure(output: &Output) -> String {
    failure_message(&output.status, &output.stderr)
}

fn failure_message(status: &std::process::ExitStatus, bytes: &[u8]) -> String {
    let message = String::from_utf8_lossy(bytes).trim().to_owned();
    if message.is_empty() {
        format!("Git exited with {status}")
    } else {
        message
    }
}

fn read_record(
    reader: &mut dyn BufRead,
    record: &mut Vec<u8>,
    operation: &str,
) -> Result<bool, Error> {
    record.clear();
    let read = reader
        .read_until(0, record)
        .map_err(|err| Error::new(ErrorKind::Unexpected, operation.to_owned()).with_source(err))?;
    if record.last() == Some(&0) {
        record.pop();
    }
    Ok(read != 0)
}

#[cfg(unix)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn encode_path(path: &Path) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}

#[cfg(unix)]
fn decode_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn decode_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}
