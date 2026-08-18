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
use std::io::BufRead;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::GitRepo;
use super::command::stderr;
use super::git_path;
use crate::Error;
use crate::ErrorKind;

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
        files: impl IntoIterator<Item = &'a Path>,
    ) -> Result<HashMap<PathBuf, GitFileHistory>, Error> {
        let selected = files
            .into_iter()
            .map(|path| {
                let relative = path
                    .strip_prefix(&self.root)
                    .expect("discovery only returns files inside the Git worktree");
                (git_path(relative), path.to_path_buf())
            })
            .collect::<HashMap<_, _>>();
        if selected.is_empty() {
            return Ok(HashMap::new());
        }

        let current_year = Timestamp::now().to_zoned(TimeZone::UTC).year();
        let started = Instant::now();
        let author = self.author_name()?;
        let mut history = self.read_history(&selected)?;
        self.apply_worktree_status(&selected, current_year, author.as_deref(), &mut history)?;

        for path in selected.values() {
            history.entry(path.clone()).or_insert_with(|| {
                let mut history = GitFileHistory::default();
                history.record_worktree(current_year, author.as_deref());
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

#[cfg(test)]
mod tests {
    use std::io::BufReader;
    use std::io::Cursor;

    use super::*;

    #[test]
    fn history_parser_handles_records_across_small_buffers() {
        let selected_path = PathBuf::from("/repo/src/main.rs");
        let selected = HashMap::from([(b"src/main.rs".to_vec(), selected_path.clone())]);
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
