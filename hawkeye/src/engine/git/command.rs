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
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Instant;

use super::GitRepo;
use crate::Error;
use crate::ErrorKind;

impl GitRepo {
    pub fn output<I, S>(&self, operation: &'static str, arguments: I) -> Result<Output, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output_unchecked(arguments)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(Error::new(ErrorKind::Unexpected, operation).with_source(stderr(&output)))
        }
    }

    pub fn output_unchecked<I, S>(&self, arguments: I) -> Result<Output, Error>
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

    pub fn read_stdout<I, S, Value>(
        &self,
        operation: &'static str,
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
        // File-backed stdin and stderr let Git consume and produce both while stdout is parsed
        // synchronously, without another thread or the risk of a full pipe blocking the child.
        let mut stderr = tempfile::tempfile().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot create Git stderr buffer").with_source(err)
        })?;
        let stderr_writer = stderr.try_clone().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot clone Git stderr buffer").with_source(err)
        })?;
        let stdin = if let Some(input) = input {
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
        let mut child = git_command(&self.root)
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
            .expect("Git stdout was configured as a pipe");
        let parsed = read(&mut BufReader::new(stdout));
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
        let source = failure(&status, &bytes);
        Err(Error::new(ErrorKind::Unexpected, operation).with_source(source))
    }
}

pub fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .env("GIT_LITERAL_PATHSPECS", "1");
    command
}

pub fn stderr(output: &Output) -> String {
    failure(&output.status, &output.stderr)
}

fn failure(status: &std::process::ExitStatus, bytes: &[u8]) -> String {
    let message = String::from_utf8_lossy(bytes).trim().to_owned();
    if message.is_empty() {
        format!("Git exited with {status}")
    } else {
        message
    }
}
