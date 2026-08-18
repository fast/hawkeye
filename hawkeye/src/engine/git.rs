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

use std::collections::BTreeSet;
use std::collections::HashMap;
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

use jiff::Timestamp;
use jiff::tz::TimeZone;

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

impl GitRepo {
    fn output<I, S>(&self, operation: &str, arguments: I) -> Result<Output, Error>
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

    fn read_stdout<I, S, Value>(
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
        let source = failure(&status, &bytes);
        Err(Error::new(ErrorKind::Unexpected, operation).with_source(source))
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

fn stderr(output: &Output) -> String {
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

#[derive(Debug, Clone, Default)]
pub struct GitFileHistory {
    pub created_year: Option<i16>,
    pub modified_year: Option<i16>,
    pub authors: BTreeSet<String>,
}

impl GitFileHistory {
    fn record(&mut self, year: i16, author: &str) {
        self.created_year = Some(self.created_year.map_or(year, |value| value.min(year)));
        self.modified_year = Some(self.modified_year.map_or(year, |value| value.max(year)));
        if !author.trim().is_empty() {
            self.authors.insert(author.to_owned());
        }
    }

    fn record_worktree(&mut self, year: i16, author: Option<&str>) {
        self.created_year.get_or_insert(year);
        self.modified_year = Some(year);
        if let Some(author) = author.filter(|value| !value.trim().is_empty()) {
            self.authors.insert(author.to_owned());
        }
    }
}

impl GitRepo {
    pub fn file_history<'a>(
        &self,
        scan_root: &Path,
        files: impl IntoIterator<Item = &'a Path>,
    ) -> Result<HashMap<PathBuf, GitFileHistory>, Error> {
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
        let selected = files
            .into_iter()
            .map(|path| (git_path(&relative_root.join(path)), path.to_path_buf()))
            .collect::<HashMap<_, _>>();
        if selected.is_empty() {
            return Ok(HashMap::new());
        }

        let worktree_year = Timestamp::now().to_zoned(TimeZone::UTC).year();
        let started = Instant::now();
        let author = self.author_name()?;
        let mut history = self.read_history(&selected)?;
        self.apply_worktree_status(&selected, worktree_year, author.as_deref(), &mut history)?;

        for path in selected.values() {
            history.entry(path.clone()).or_insert_with(|| {
                let mut history = GitFileHistory::default();
                history.record_worktree(worktree_year, author.as_deref());
                history
            });
        }
        log::debug!(
            "resolved Git file history for {} files in {:?}",
            selected.len(),
            started.elapsed()
        );
        Ok(history)
    }

    fn has_head(&self) -> Result<bool, Error> {
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

    fn author_name(&self) -> Result<Option<String>, Error> {
        let output = self.output_unchecked(["config", "--get", "user.name"])?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(
                Error::new(ErrorKind::Unexpected, "cannot read Git author name")
                    .with_source(stderr(&output)),
            );
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok((!value.is_empty()).then_some(value))
    }

    fn read_history(
        &self,
        selected: &HashMap<Vec<u8>, PathBuf>,
    ) -> Result<HashMap<PathBuf, GitFileHistory>, Error> {
        if !self.has_head()? {
            return Ok(HashMap::new());
        }
        let mut arguments = vec![
            "-c",
            "core.quotepath=false",
            "log",
            "--full-history",
            "--no-merges",
            "--no-renames",
            "--format=%x00%x00%cI%x00%an",
            "--name-only",
            "-z",
        ];
        let pathspecs = history_pathspecs(selected);
        if pathspecs.is_some() {
            arguments.push("--stdin");
        } else {
            // `git log --stdin` is line-delimited. Fall back to an unfiltered traversal for the
            // rare repository containing a selected path with an embedded line ending.
            arguments.push("--");
        }
        // Empty NUL-delimited records cannot be file paths, so a pair unambiguously frames commit
        // metadata without relying on Git's quoting rules or reserving a valid path byte.
        self.read_stdout(
            "cannot read Git history",
            arguments,
            pathspecs.as_deref(),
            |reader| parse_history(reader, selected),
        )
    }

    fn apply_worktree_status(
        &self,
        selected: &HashMap<Vec<u8>, PathBuf>,
        year: i16,
        author: Option<&str>,
        history: &mut HashMap<PathBuf, GitFileHistory>,
    ) -> Result<(), Error> {
        let output = self.output(
            "cannot inspect Git worktree status",
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        let mut records = output.stdout.split(|byte| *byte == 0);
        while let Some(record) = records.next() {
            if record.len() < 4 {
                continue;
            }
            let status = &record[..2];
            if let Some(selected_path) = selected.get(&record[3..]) {
                history
                    .entry(selected_path.clone())
                    .or_default()
                    .record_worktree(year, author);
            }
            if status.contains(&b'R') || status.contains(&b'C') {
                records.next();
            }
        }
        Ok(())
    }
}

fn history_pathspecs(selected: &HashMap<Vec<u8>, PathBuf>) -> Option<Vec<u8>> {
    let mut input = b"--\n".to_vec();
    for path in selected.keys() {
        if path.contains(&b'\n') || path.contains(&b'\r') {
            return None;
        }
        input.extend_from_slice(path);
        input.push(b'\n');
    }
    Some(input)
}

fn parse_history(
    reader: &mut dyn BufRead,
    selected: &HashMap<Vec<u8>, PathBuf>,
) -> Result<HashMap<PathBuf, GitFileHistory>, Error> {
    #[derive(Clone, Copy)]
    enum State {
        Paths,
        Author(Option<i16>),
    }

    let mut current: Option<(i16, String)> = None;
    let mut expecting_first_path = false;
    let mut empty_records = 0;
    let mut state = State::Paths;
    let mut result = HashMap::<PathBuf, GitFileHistory>::new();
    let mut record = Vec::new();
    loop {
        record.clear();
        let read = reader.read_until(0, &mut record).map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot read Git history").with_source(err)
        })?;
        if read == 0 {
            break;
        }
        if record.last() == Some(&0) {
            record.pop();
        }
        let record = record.as_slice();
        if record.is_empty() {
            empty_records += 1;
            continue;
        }
        if matches!(state, State::Paths) && empty_records >= 2 {
            let year = record
                .get(..4)
                .and_then(|year| std::str::from_utf8(year).ok())
                .and_then(|year| year.parse::<i16>().ok());
            current = None;
            expecting_first_path = false;
            empty_records = 0;
            state = State::Author(year);
            continue;
        }
        if let State::Author(year) = state {
            current = year.map(|year| (year, String::from_utf8_lossy(record).into_owned()));
            expecting_first_path = true;
            empty_records = 0;
            state = State::Paths;
            continue;
        }
        empty_records = 0;
        let path = if expecting_first_path {
            expecting_first_path = false;
            // `--name-only -z` inserts one newline between the pretty header and its first path.
            record.strip_prefix(b"\n").unwrap_or(record)
        } else {
            record
        };
        if path.is_empty() {
            continue;
        }
        let Some(path) = selected.get(path) else {
            continue;
        };
        if let Some((year, author)) = &current {
            result
                .entry(path.clone())
                .or_default()
                .record(*year, author);
        }
    }
    Ok(result)
}
