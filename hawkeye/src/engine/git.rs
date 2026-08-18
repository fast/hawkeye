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
use std::convert::Infallible;
use std::ops::ControlFlow;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use gix::bstr::BStr;
use gix::bstr::BString;
use gix::bstr::ByteSlice;
use jiff::Timestamp;
use jiff::tz::Offset;
use jiff::tz::TimeZone;

use crate::Error;
use crate::ErrorKind;

#[derive(Debug, Clone, Default)]
pub struct FileHistory {
    pub created_year: Option<i16>,
    pub modified_year: Option<i16>,
    pub authors: BTreeSet<String>,
}

impl FileHistory {
    fn record_commit(&mut self, year: i16, author: &str) {
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

pub struct Repository {
    inner: gix::Repository,
    root: PathBuf,
}

impl Repository {
    pub fn discover(root: &Path) -> Result<Self, Error> {
        let started = Instant::now();
        let inner = gix::discover(root).map_err(|err| {
            let (kind, message) = match &err {
                gix::discover::Error::Discover(
                    gix::discover::upwards::Error::NoGitRepository { .. }
                    | gix::discover::upwards::Error::NoGitRepositoryWithinCeiling { .. }
                    | gix::discover::upwards::Error::NoGitRepositoryWithinFs { .. },
                ) => (
                    ErrorKind::Unsupported,
                    format!("{} is not a usable Git worktree", root.display()),
                ),
                _ => (
                    ErrorKind::Unexpected,
                    format!("cannot open Git repository for {}", root.display()),
                ),
            };
            Error::new(kind, message).with_source(err)
        })?;
        let workdir = inner.workdir().ok_or_else(|| {
            Error::new(
                ErrorKind::Unsupported,
                format!("{} is not a usable Git worktree", root.display()),
            )
        })?;
        let root = workdir.canonicalize().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot resolve repository root").with_source(err)
        })?;
        log::debug!(
            "discovered Git repository {} in {:?}",
            root.display(),
            started.elapsed()
        );
        Ok(Self { inner, root })
    }

    pub fn list_files(&self, scan_root: &Path) -> Result<BTreeSet<PathBuf>, Error> {
        let relative_root = self.relative_root(scan_root)?;
        let prefix = path_prefix(relative_root);
        let index = self.inner.index_or_empty().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot read Git index").with_source(err)
        })?;
        let mut files = BTreeSet::new();

        if let Some(entries) = index.prefixed_entries(prefix.as_bstr()) {
            for entry in entries {
                if let Some(path) = self.worktree_file(entry.path(&index), scan_root)? {
                    files.insert(path);
                }
            }
        }

        let options = self
            .inner
            .dirwalk_options()
            .map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    "cannot configure Git worktree traversal",
                )
                .with_source(err)
            })?
            .emit_untracked(gix::dir::walk::EmissionMode::Matching);
        let mut untracked = self
            .inner
            .dirwalk_iter(
                index,
                scan_pathspec(relative_root),
                Default::default(),
                options,
            )
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot list Git worktree files").with_source(err)
            })?;
        for item in &mut untracked {
            let item = item.map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot list Git worktree files").with_source(err)
            })?;
            if let Some(path) = self.worktree_file(item.entry.rela_path.as_ref(), scan_root)? {
                files.insert(path);
            }
        }

        Ok(files)
    }

    pub fn file_history<'a>(
        &self,
        scan_root: &Path,
        files: impl IntoIterator<Item = &'a Path>,
    ) -> Result<HashMap<PathBuf, FileHistory>, Error> {
        let relative_root = self.relative_root(scan_root)?;
        let selected = files
            .into_iter()
            .map(|path| (encode_path(&relative_root.join(path)), path.to_path_buf()))
            .collect::<HashMap<_, _>>();
        if selected.is_empty() {
            return Ok(HashMap::new());
        }

        let started = Instant::now();
        let current_year = Timestamp::now().to_zoned(TimeZone::UTC).year();
        let current_author = self
            .inner
            .config_snapshot()
            .string(gix::config::tree::User::NAME)
            .map(|value| String::from_utf8_lossy(&value).trim().to_owned())
            .filter(|value| !value.is_empty());
        let mut history = self.committed_history(&selected)?;

        let mut status = self
            .inner
            .status(gix::progress::Discard)
            .map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    "cannot configure Git worktree status",
                )
                .with_source(err)
            })?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .index_worktree_submodules(None)
            .index_worktree_rewrites(None)
            .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled)
            .into_iter(scan_pathspec(relative_root))
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot inspect Git worktree status")
                    .with_source(err)
            })?;
        for item in &mut status {
            let item = item.map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot inspect Git worktree status")
                    .with_source(err)
            })?;
            let path = match &item {
                gix::status::Item::IndexWorktree(item) if item.summary().is_some() => {
                    item.rela_path()
                }
                gix::status::Item::TreeIndex(change) => change.location(),
                _ => continue,
            };
            if let Some(path) = selected.get(path) {
                history
                    .entry(path.clone())
                    .or_default()
                    .record_worktree(current_year, current_author.as_deref());
            }
        }

        // Files absent from HEAD are new worktree files even when status omits them, for example
        // when the caller explicitly selected an ignored file.
        for path in selected.values() {
            history.entry(path.clone()).or_insert_with(|| {
                let mut history = FileHistory::default();
                history.record_worktree(current_year, current_author.as_deref());
                history
            });
        }
        log::debug!(
            "resolved Git history for {} files in {:?}",
            selected.len(),
            started.elapsed()
        );
        Ok(history)
    }

    pub fn is_shallow(&self) -> bool {
        self.inner.is_shallow()
    }

    fn committed_history(
        &self,
        selected: &HashMap<BString, PathBuf>,
    ) -> Result<HashMap<PathBuf, FileHistory>, Error> {
        let mut repo = self.inner.clone();
        // Repeated tree diffs benefit from decoded objects, but the recommendation scales with
        // the complete index. Cap this private history handle for predictable memory use.
        let cache_size = {
            let index = repo.index_or_empty().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git index").with_source(err)
            })?;
            let cache_size = repo.compute_object_cache_size_for_tree_diffs(&index);
            cache_size.min(16 * 1024 * 1024) // up to 16 MiB for a large repository
        };
        repo.object_cache_size_if_unset(cache_size);

        let head = repo
            .head()
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot inspect Git HEAD").with_source(err)
            })?
            .try_into_peeled_id()
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot inspect Git HEAD").with_source(err)
            })?;
        let Some(head) = head else {
            return Ok(HashMap::new());
        };

        let mut resource_cache = repo
            .diff_resource_cache(gix::diff::blob::pipeline::Mode::ToGit, Default::default())
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot prepare Git tree diff").with_source(err)
            })?;
        let commits = repo.rev_walk([head.detach()]).all().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot traverse Git history").with_source(err)
        })?;
        let mut history = HashMap::<PathBuf, FileHistory>::new();
        for info in commits {
            let info = info.map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot traverse Git history").with_source(err)
            })?;
            // A merge has multiple possible before states. Its parents already carry the file
            // changes, so skipping the merge avoids assigning the merge author to those changes.
            let parent = match info.parent_ids.as_slice() {
                [] => None,
                [parent] => Some(*parent),
                _ => continue,
            };
            let commit = info.object().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit").with_source(err)
            })?;
            let year = commit_year(commit.time().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit time").with_source(err)
            })?)?;
            let author = commit.author().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit author").with_source(err)
            })?;
            let author = String::from_utf8_lossy(author.name.as_ref()).into_owned();
            let tree = commit.tree().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit tree").with_source(err)
            })?;
            let previous_tree = if let Some(parent) = parent {
                repo.find_commit(parent)
                    .map_err(|err| {
                        Error::new(ErrorKind::Unexpected, "cannot read Git parent commit")
                            .with_source(err)
                    })?
                    .tree()
                    .map_err(|err| {
                        Error::new(ErrorKind::Unexpected, "cannot read Git parent tree")
                            .with_source(err)
                    })?
            } else {
                repo.empty_tree()
            };
            let mut changes = previous_tree.changes().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot configure Git tree diff").with_source(err)
            })?;
            // A rename is a removal and an addition for header-history purposes. Avoid the much
            // more expensive similarity analysis and let the new path start its own history.
            changes.options(|options| {
                options.track_rewrites(None);
            });
            changes
                .for_each_to_obtain_tree_with_cache::<Infallible>(
                    &tree,
                    &mut resource_cache,
                    |change| {
                        if let Some(path) = selected.get(change.location()) {
                            history
                                .entry(path.clone())
                                .or_default()
                                .record_commit(year, &author);
                        }
                        Ok(ControlFlow::Continue(()))
                    },
                )
                .map_err(|err| {
                    Error::new(ErrorKind::Unexpected, "cannot compare Git commit trees")
                        .with_source(err)
                })?;
        }
        Ok(history)
    }

    fn relative_root<'a>(&self, scan_root: &'a Path) -> Result<&'a Path, Error> {
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

    fn worktree_file(
        &self,
        repository_path: &BStr,
        scan_root: &Path,
    ) -> Result<Option<PathBuf>, Error> {
        let path = self.root.join(gix::path::from_bstr(repository_path));
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read metadata for {}", path.display()),
                )
                .with_source(err));
            }
        };
        let file_type = metadata.file_type();
        if !file_type.is_file() && !(file_type.is_symlink() && path.is_file()) {
            return Ok(None);
        }
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
        Ok(Some(relative.to_path_buf()))
    }
}

fn path_prefix(path: &Path) -> BString {
    let mut path = encode_path(path);
    if !path.is_empty() {
        path.push(b'/');
    }
    path
}

fn scan_pathspec(relative_root: &Path) -> Option<BString> {
    let prefix = path_prefix(relative_root);
    if prefix.is_empty() {
        return None;
    }

    let mut pattern = BString::from(":(top,literal)");
    pattern.extend_from_slice(&prefix);
    Some(pattern)
}

fn encode_path(path: &Path) -> BString {
    gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path)).into_owned()
}

fn commit_year(time: gix::date::Time) -> Result<i16, Error> {
    let timestamp = Timestamp::from_second(time.seconds).map_err(|err| {
        Error::new(
            ErrorKind::Unexpected,
            "Git commit time is outside the supported range",
        )
        .with_source(err)
    })?;
    let offset = Offset::from_seconds(time.offset).map_err(|err| {
        Error::new(
            ErrorKind::Unexpected,
            "Git commit timezone is outside the supported range",
        )
        .with_source(err)
    })?;
    Ok(timestamp.to_zoned(offset.to_time_zone()).year())
}
