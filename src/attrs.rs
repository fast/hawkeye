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
use std::process::Command;
use std::process::Output;
use std::time::SystemTime;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::Error;
use crate::Result;
use crate::config::FeatureMode;

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
    pub(crate) fn new(root: &Path, files: &[PathBuf], mode: FeatureMode) -> Result<Self> {
        let current_year = utc_year(SystemTime::now())
            .ok_or_else(|| Error::Git("the current UTC year is out of range".to_owned()))?;
        if mode == FeatureMode::Disable {
            return Ok(Self {
                git_enabled: false,
                git: BTreeMap::new(),
            });
        }

        let Some(repo_root) = discover_repository(root, mode)? else {
            return Ok(Self {
                git_enabled: false,
                git: BTreeMap::new(),
            });
        };

        let selected = files
            .iter()
            .filter_map(|path| {
                path.strip_prefix(&repo_root)
                    .ok()
                    .map(|relative| (git_path(relative), path.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut git = read_history(&repo_root, &selected)?;
        apply_worktree_status(&repo_root, &selected, current_year, &mut git)?;

        let author = current_git_author(&repo_root)?;
        for path in selected.values() {
            if !git.contains_key(path) {
                git.entry(path.clone()).or_default().record_worktree(
                    current_year,
                    author.as_deref(),
                    true,
                );
            }
        }

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

fn discover_repository(root: &Path, mode: FeatureMode) -> Result<Option<PathBuf>> {
    let output = match Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        Ok(output) => output,
        Err(error) if mode == FeatureMode::Auto && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(Error::Git(format!("cannot start Git: {error}"))),
    };
    if !output.status.success() {
        if mode == FeatureMode::Auto {
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
    PathBuf::from(path)
        .canonicalize()
        .map(Some)
        .map_err(|error| Error::Git(format!("cannot resolve repository root: {error}")))
}

fn read_history(
    repo_root: &Path,
    selected: &BTreeMap<String, PathBuf>,
) -> Result<BTreeMap<PathBuf, GitAttrs>> {
    const MARKER: char = '\u{001e}';
    if !git_has_head(repo_root)? {
        return Ok(BTreeMap::new());
    }
    let output = git_output(
        repo_root,
        [
            "-c",
            "core.quotepath=false",
            "log",
            "--full-history",
            "--no-merges",
            "--no-renames",
            "--format=\u{001e}%cI\t%an\t%ae",
            "--name-only",
            "--",
        ],
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut current: Option<(i16, String)> = None;
    let mut result = BTreeMap::<PathBuf, GitAttrs>::new();
    for line in stdout.lines() {
        if let Some(header) = line.strip_prefix(MARKER) {
            let mut fields = header.splitn(3, '\t');
            let year = fields
                .next()
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse::<i16>().ok());
            let name = fields.next().unwrap_or_default();
            let _email = fields.next().unwrap_or_default();
            current = year.map(|year| (year, name.to_owned()));
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let Some(path) = selected.get(line) else {
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
    repo_root: &Path,
    selected: &BTreeMap<String, PathBuf>,
    year: i16,
    attrs: &mut BTreeMap<PathBuf, GitAttrs>,
) -> Result<()> {
    let output = git_output(
        repo_root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let author = current_git_author(repo_root)?;
    let records = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.len() < 4 {
            continue;
        }
        let status = &record[..2];
        let path = String::from_utf8_lossy(&record[3..]).into_owned();
        let new_file = status == b"??" || status.contains(&b'A');
        if let Some(selected_path) = selected.get(&path) {
            attrs
                .entry(selected_path.clone())
                .or_default()
                .record_worktree(year, author.as_deref(), new_file);
        }
        if status.contains(&b'R') || status.contains(&b'C') {
            index += 1;
        }
    }
    Ok(())
}

fn current_git_author(repo_root: &Path) -> Result<Option<String>> {
    git_optional_config(repo_root, "user.name")
}

fn git_has_head(repo_root: &Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(repo_root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .map_err(|error| Error::Git(format!("cannot inspect Git HEAD: {error}")))?;
    Ok(output.status.success())
}

fn git_optional_config(repo_root: &Path, key: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(repo_root)
        .args(["config", "--get", key])
        .output()
        .map_err(|error| Error::Git(format!("cannot read {key}: {error}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

fn git_output<'argument>(
    repo_root: &Path,
    arguments: impl IntoIterator<Item = &'argument str>,
) -> Result<Output> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(repo_root)
        .args(arguments)
        .output()
        .map_err(|error| Error::Git(format!("cannot start Git: {error}")))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(Error::Git(stderr(&output)))
    }
}

fn stderr(output: &Output) -> String {
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if message.is_empty() {
        format!("Git exited with {}", output.status)
    } else {
        message
    }
}

fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn utc_year(time: SystemTime) -> Option<i16> {
    Timestamp::try_from(time)
        .ok()
        .map(|timestamp| timestamp.to_zoned(TimeZone::UTC).year())
}
