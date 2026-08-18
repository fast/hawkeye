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

use gix::bstr::BString;
use jiff::Timestamp;
use jiff::tz::Offset;
use jiff::tz::TimeZone;

use super::Repository;
use super::encode_path;
use super::scan_pathspec;
use crate::Error;
use crate::ErrorKind;

const OBJECT_CACHE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct FileHistory {
    pub created_year: Option<i16>,
    pub modified_year: Option<i16>,
    pub authors: BTreeSet<String>,
}

impl FileHistory {
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

impl Repository {
    pub fn file_history<'a>(
        &self,
        scan_root: &Path,
        files: impl IntoIterator<Item = &'a Path>,
    ) -> Result<HashMap<PathBuf, FileHistory>, Error> {
        let relative_root = self.relative_scan_root(scan_root)?;
        let selected = files
            .into_iter()
            .map(|path| (encode_path(&relative_root.join(path)), path.to_path_buf()))
            .collect::<HashMap<_, _>>();
        if selected.is_empty() {
            return Ok(HashMap::new());
        }

        let worktree_year = Timestamp::now().to_zoned(TimeZone::UTC).year();
        let started = Instant::now();
        let author = self.author_name();
        let mut history = self.read_history(&selected)?;
        self.apply_worktree_status(
            relative_root,
            &selected,
            worktree_year,
            author.as_deref(),
            &mut history,
        )?;

        for path in selected.values() {
            history.entry(path.clone()).or_insert_with(|| {
                let mut history = FileHistory::default();
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

    fn author_name(&self) -> Option<String> {
        self.inner
            .config_snapshot()
            .string(gix::config::tree::User::NAME)
            .map(|value| String::from_utf8_lossy(&value).trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn read_history(
        &self,
        selected: &HashMap<BString, PathBuf>,
    ) -> Result<HashMap<PathBuf, FileHistory>, Error> {
        let mut repo = self.inner.clone();
        // Repeated tree diffs benefit from decoded objects, but gix's recommendation scales with
        // the complete index. Cap this private history handle for predictable memory use.
        let cache_size = {
            let index = repo.index_or_empty().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git index").with_source(err)
            })?;
            repo.compute_object_cache_size_for_tree_diffs(&index)
                .min(OBJECT_CACHE_LIMIT)
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
            let parent = match info.parent_ids.as_slice() {
                [] => None,
                [parent] => Some(*parent),
                _ => continue,
            };
            let commit = info.object().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit").with_source(err)
            })?;
            let time = commit.time().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit time").with_source(err)
            })?;
            let year = commit_year(time)?;
            let author = commit.author().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit author").with_source(err)
            })?;
            let author = String::from_utf8_lossy(author.name.as_ref()).into_owned();
            let tree = commit.tree().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit tree").with_source(err)
            })?;
            let previous_tree = if let Some(parent) = parent {
                let parent = repo.find_commit(parent).map_err(|err| {
                    Error::new(ErrorKind::Unexpected, "cannot read Git parent commit")
                        .with_source(err)
                })?;
                parent.tree().map_err(|err| {
                    Error::new(ErrorKind::Unexpected, "cannot read Git parent tree")
                        .with_source(err)
                })?
            } else {
                repo.empty_tree()
            };
            let mut changes = previous_tree.changes().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot configure Git tree diff").with_source(err)
            })?;
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
                                .record(year, &author);
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

    fn apply_worktree_status(
        &self,
        relative_root: &Path,
        selected: &HashMap<BString, PathBuf>,
        year: i16,
        author: Option<&str>,
        history: &mut HashMap<PathBuf, FileHistory>,
    ) -> Result<(), Error> {
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
                    .record_worktree(year, author);
            }
        }
        Ok(())
    }
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
