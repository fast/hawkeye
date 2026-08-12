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
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;
use std::time::SystemTime;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::Error;
use crate::Result;
use crate::config::FeatureMode;
use crate::git::GitRepo;
use crate::git::git_path;

/// Per-file values exposed to MiniJinja as `attrs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileAttrs {
    /// The basename of the current source file.
    pub filename: String,
    /// The filesystem creation year, when the platform exposes one.
    pub disk_file_created_year: Option<i16>,
    /// The filesystem modification year.
    pub disk_file_modified_year: Option<i16>,
    /// The earliest commit year associated with the file.
    pub git_file_created_year: Option<i16>,
    /// The latest commit year, or the current year for a dirty file.
    pub git_file_modified_year: Option<i16>,
    /// Distinct Git commit authors associated with the file.
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

    fn record_worktree(&mut self, year: i16, author: Option<&str>, new_file: bool) {
        if new_file || self.created_year.is_none() {
            self.created_year = Some(year);
        }
        self.modified_year = Some(year);
        if let Some(author) = author.filter(|value| !value.trim().is_empty()) {
            self.authors.insert(author.to_owned());
        }
    }
}

pub(crate) struct FileAttrsResolver {
    git_enabled: bool,
    git: BTreeMap<PathBuf, GitAttrs>,
}

impl FileAttrsResolver {
    pub(crate) fn new(
        files: &[PathBuf],
        mode: FeatureMode,
        repo: Option<&GitRepo>,
    ) -> Result<Self> {
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
        if repo.is_shallow()? {
            let message = "Git file attributes require complete history, but the repository is shallow; fetch complete history first";
            if mode == FeatureMode::Auto {
                log::warn!("{message}; continuing with Git file attributes disabled");
                return Ok(Self {
                    git_enabled: false,
                    git: BTreeMap::new(),
                });
            }
            return Err(Error::Git(message.to_owned()));
        }
        let current_year = utc_year(SystemTime::now())
            .ok_or_else(|| Error::Git("the current UTC year is out of range".to_owned()))?;
        let started = Instant::now();

        let selected = files
            .iter()
            .filter_map(|path| {
                path.strip_prefix(repo.root())
                    .ok()
                    .map(|relative| (git_path(relative), path.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let author = current_git_author(repo)?;
        let mut git = read_history(repo, &selected)?;
        apply_worktree_status(repo, &selected, current_year, author.as_deref(), &mut git)?;

        for path in selected.values() {
            if !git.contains_key(path) {
                git.entry(path.clone()).or_default().record_worktree(
                    current_year,
                    author.as_deref(),
                    true,
                );
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

    pub(crate) fn for_file(&self, path: &Path) -> Result<FileAttrs> {
        let metadata =
            fs::metadata(path).map_err(|source| Error::io("read metadata for", path, source))?;
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
) -> Result<BTreeMap<PathBuf, GitAttrs>> {
    const MARKER: u8 = 0x1e;
    if !repo.has_head()? {
        return Ok(BTreeMap::new());
    }
    // NUL separators preserve arbitrary path bytes. The record marker keeps commit metadata
    // distinguishable from a path without relying on Git's quoting rules.
    let output = repo.output([
        "-c",
        "core.quotepath=false",
        "log",
        "--full-history",
        "--no-merges",
        "--no-renames",
        "--format=\u{001e}%cI%x00%an",
        "--name-only",
        "-z",
        "--",
    ])?;
    let mut current: Option<(i16, String)> = None;
    let mut expecting_author = false;
    let mut expecting_first_path = false;
    let mut result = BTreeMap::<PathBuf, GitAttrs>::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if let Some(date) = record.strip_prefix(&[MARKER]) {
            let year = date
                .get(..4)
                .and_then(|year| std::str::from_utf8(year).ok())
                .and_then(|year| year.parse::<i16>().ok());
            current = year.map(|year| (year, String::new()));
            expecting_author = true;
            expecting_first_path = false;
            continue;
        }
        if expecting_author {
            if let Some((_, author)) = &mut current {
                *author = String::from_utf8_lossy(record).into_owned();
            }
            expecting_author = false;
            expecting_first_path = true;
            continue;
        }
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
) -> Result<()> {
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
        let new_file = status == b"??" || status.contains(&b'A');
        if let Some(selected_path) = selected.get(&record[3..]) {
            attrs
                .entry(selected_path.clone())
                .or_default()
                .record_worktree(year, author, new_file);
        }
        if status.contains(&b'R') || status.contains(&b'C') {
            index += 1;
        }
    }
    Ok(())
}

fn current_git_author(repo: &GitRepo) -> Result<Option<String>> {
    repo.optional_config("user.name")
}

fn utc_year(time: SystemTime) -> Option<i16> {
    Timestamp::try_from(time)
        .ok()
        .map(|timestamp| timestamp.to_zoned(TimeZone::UTC).year())
}
