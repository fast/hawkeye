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

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::BufRead;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;
use std::time::SystemTime;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::Error;
use crate::ErrorKind;
use crate::config::FeatureMode;
use crate::git::GitRepo;
use crate::git::git_path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileAttrs {
    pub filename: String,
    pub disk_file_created_year: Option<i16>,
    pub disk_file_modified_year: Option<i16>,
    pub git_file_created_year: Option<i16>,
    pub git_file_modified_year: Option<i16>,
    pub git_authors: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct GitAttrs {
    created_year: Option<i16>,
    modified_year: Option<i16>,
    authors: BTreeSet<String>,
}

impl GitAttrs {
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

pub struct FileAttrsResolver {
    git_enabled: bool,
    git: BTreeMap<PathBuf, GitAttrs>,
}

impl FileAttrsResolver {
    pub fn new<'a>(
        files: impl IntoIterator<Item = &'a Path>,
        mode: FeatureMode,
        repo: Option<&GitRepo>,
    ) -> Result<Self, Error> {
        if mode == FeatureMode::Disable {
            return Ok(Self {
                git_enabled: false,
                git: BTreeMap::new(),
            });
        }

        let Some(repo) = repo else {
            return Ok(Self {
                git_enabled: false,
                git: BTreeMap::new(),
            });
        };

        let selected = files
            .into_iter()
            .filter_map(|path| {
                path.strip_prefix(&repo.root)
                    .ok()
                    .map(|relative| (git_path(relative), path.to_path_buf()))
            })
            .collect::<BTreeMap<_, _>>();
        if selected.is_empty() {
            return Ok(Self {
                git_enabled: true,
                git: BTreeMap::new(),
            });
        }
        if repo.is_shallow()? {
            let message = "Git file attributes require complete history, but the repository is shallow; fetch complete history first";
            if mode == FeatureMode::Auto {
                log::warn!("{message}; continuing with Git file attributes disabled");
                return Ok(Self {
                    git_enabled: false,
                    git: BTreeMap::new(),
                });
            }
            return Err(Error::new(ErrorKind::Unsupported, message));
        }
        let current_year = utc_year(SystemTime::now()).ok_or_else(|| {
            Error::new(
                ErrorKind::Unexpected,
                "the current UTC year is out of range",
            )
        })?;
        let started = Instant::now();

        let author = repo.optional_config("user.name")?;
        let mut git = read_history(repo, &selected)?;
        apply_worktree_status(repo, &selected, current_year, author.as_deref(), &mut git)?;

        for path in selected.values() {
            if !git.contains_key(path) {
                git.entry(path.clone())
                    .or_default()
                    .record_worktree(current_year, author.as_deref());
            }
        }
        log::debug!(
            "resolved Git attributes for {} files in {:?}",
            selected.len(),
            started.elapsed()
        );

        Ok(Self {
            git_enabled: true,
            git,
        })
    }

    pub fn for_file(&self, path: &Path) -> Result<FileAttrs, Error> {
        let metadata = fs::metadata(path).map_err(|err| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot read metadata for {}", path.display()),
            )
            .with_source(err)
        })?;
        let git = self.git.get(path);
        Ok(FileAttrs {
            filename: path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default(),
            disk_file_created_year: metadata.created().ok().and_then(utc_year),
            disk_file_modified_year: metadata.modified().ok().and_then(utc_year),
            git_file_created_year: self
                .git_enabled
                .then(|| git.and_then(|value| value.created_year))
                .flatten(),
            git_file_modified_year: self
                .git_enabled
                .then(|| git.and_then(|value| value.modified_year))
                .flatten(),
            git_authors: git
                .map(|value| value.authors.iter().cloned().collect())
                .unwrap_or_default(),
        })
    }
}

fn read_history(
    repo: &GitRepo,
    selected: &BTreeMap<Vec<u8>, PathBuf>,
) -> Result<BTreeMap<PathBuf, GitAttrs>, Error> {
    if !repo.has_head()? {
        return Ok(BTreeMap::new());
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
        // `git log --stdin` is line-delimited. Fall back to an unfiltered traversal for the rare
        // repository containing a selected path with an embedded line ending.
        arguments.push("--");
    }
    // Empty NUL-delimited records cannot be file paths, so a pair unambiguously frames commit
    // metadata without relying on Git's quoting rules or reserving a valid path byte.
    repo.read_stdout(arguments, pathspecs.as_deref(), |reader| {
        parse_history(reader, selected)
    })
}

fn history_pathspecs(selected: &BTreeMap<Vec<u8>, PathBuf>) -> Option<Vec<u8>> {
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
    selected: &BTreeMap<Vec<u8>, PathBuf>,
) -> Result<BTreeMap<PathBuf, GitAttrs>, Error> {
    #[derive(Clone, Copy)]
    enum State {
        Paths,
        Author(Option<i16>),
    }

    let mut current: Option<(i16, String)> = None;
    let mut expecting_first_path = false;
    let mut empty_records = 0;
    let mut state = State::Paths;
    let mut result = BTreeMap::<PathBuf, GitAttrs>::new();
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

fn apply_worktree_status(
    repo: &GitRepo,
    selected: &BTreeMap<Vec<u8>, PathBuf>,
    year: i16,
    author: Option<&str>,
    attrs: &mut BTreeMap<PathBuf, GitAttrs>,
) -> Result<(), Error> {
    let output = repo.output(["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let records = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.len() < 4 {
            continue;
        }
        let status = &record[..2];
        if let Some(selected_path) = selected.get(&record[3..]) {
            attrs
                .entry(selected_path.clone())
                .or_default()
                .record_worktree(year, author);
        }
        if status.contains(&b'R') || status.contains(&b'C') {
            index += 1;
        }
    }
    Ok(())
}

fn utc_year(time: SystemTime) -> Option<i16> {
    Timestamp::try_from(time)
        .ok()
        .map(|timestamp| timestamp.to_zoned(TimeZone::UTC).year())
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;
    use std::io::Cursor;

    use super::*;

    #[test]
    fn history_parser_handles_records_across_small_buffers() {
        let selected_path = PathBuf::from("/repo/src/main.rs");
        let selected = BTreeMap::from([(b"src/main.rs".to_vec(), selected_path.clone())]);
        let history = b"\x00\x002024-01-02T00:00:00Z\x00Alice\x00\nsrc/main.rs\x00other.rs\x00\x00\x002020-03-04T00:00:00Z\x00Bob\x00\nsrc/main.rs\x00";
        let mut reader = BufReader::with_capacity(3, Cursor::new(history));

        let attrs = parse_history(&mut reader, &selected).expect("parse history");
        let attrs = attrs.get(&selected_path).expect("selected file attributes");
        assert_eq!(attrs.created_year, Some(2020));
        assert_eq!(attrs.modified_year, Some(2024));
        assert_eq!(
            attrs.authors,
            BTreeSet::from(["Alice".to_owned(), "Bob".to_owned()])
        );
    }
}
