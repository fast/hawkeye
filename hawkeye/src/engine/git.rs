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
use std::collections::HashSet;
use std::collections::VecDeque;
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

use crate::Error;
use crate::ErrorKind;

#[derive(Debug, Clone, Default)]
pub struct FileHistory {
    pub created_year: Option<i16>,
    pub modified_year: Option<i16>,
    pub authors: BTreeSet<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PathChange {
    Addition,
    Modification,
}

// A dense path ID keeps merge de-duplication to one bit per selected path and visited commit.
// Retaining full path strings here would make memory grow prohibitively on large repositories.
struct VisitedPaths {
    words_per_commit: usize,
    by_commit: HashMap<gix::ObjectId, Box<[u64]>>,
}

impl VisitedPaths {
    fn new(path_count: usize) -> Self {
        debug_assert_ne!(path_count, 0);
        Self {
            words_per_commit: path_count.div_ceil(u64::BITS as usize),
            by_commit: HashMap::new(),
        }
    }

    fn insert(&mut self, commit: gix::ObjectId, path: usize) -> bool {
        let words = self
            .by_commit
            .entry(commit)
            .or_insert_with(|| vec![0; self.words_per_commit].into_boxed_slice());
        let word = &mut words[path / u64::BITS as usize];
        let mask = 1_u64 << (path % u64::BITS as usize);
        let inserted = *word & mask == 0;
        *word |= mask;
        inserted
    }
}

impl FileHistory {
    fn record_change(&mut self, year: i16, author: &str) {
        self.modified_year = Some(self.modified_year.map_or(year, |value| value.max(year)));
        if !author.trim().is_empty() {
            self.authors.insert(author.to_owned());
        }
    }

    fn record_addition(&mut self, year: i16, author: &str) {
        self.created_year = Some(self.created_year.map_or(year, |value| value.min(year)));
        self.record_change(year, author);
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
        let mut selected = HashMap::new();
        let mut files_by_id = Vec::new();
        for path in files {
            let git_path = encode_path(&relative_root.join(path));
            if selected.contains_key(git_path.as_bstr()) {
                continue;
            }
            let id = files_by_id.len();
            selected.insert(git_path, id);
            files_by_id.push(path.to_path_buf());
        }
        if files_by_id.is_empty() {
            return Ok(HashMap::new());
        }

        let started = Instant::now();
        let current_year = commit_year(gix::date::Time::now_local_or_utc())?;
        let current_author = self
            .inner
            .config_snapshot()
            .string(gix::config::tree::User::NAME)
            .map(|value| String::from_utf8_lossy(&value).trim().to_owned())
            .filter(|value| !value.is_empty());
        let mut history = self.committed_history(&selected, &files_by_id)?;
        let status_pathspecs = selected
            .keys()
            .map(|path| literal_pathspec(path.as_bstr()))
            .collect::<Vec<_>>();

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
            .into_iter(status_pathspecs)
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
            if let Some(id) = selected.get(path) {
                let path = &files_by_id[*id];
                history
                    .entry(path.clone())
                    .or_default()
                    .record_worktree(current_year, current_author.as_deref());
            }
        }

        // Files absent from HEAD are new worktree files even when status omits them, for example
        // when the caller explicitly selected an ignored file.
        for path in &files_by_id {
            history.entry(path.clone()).or_insert_with(|| {
                let mut history = FileHistory::default();
                history.record_worktree(current_year, current_author.as_deref());
                history
            });
        }
        log::debug!(
            "resolved Git history for {} files in {:?}",
            files_by_id.len(),
            started.elapsed()
        );
        Ok(history)
    }

    pub fn is_shallow(&self) -> bool {
        self.inner.is_shallow()
    }

    fn committed_history(
        &self,
        selected: &HashMap<BString, usize>,
        files_by_id: &[PathBuf],
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

        let head = head.detach();
        let head_tree = repo
            .find_commit(head)
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git HEAD").with_source(err)
            })?
            .tree()
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git HEAD tree").with_source(err)
            })?;
        let mut pending = HashSet::with_capacity(selected.len());
        for (path, id) in selected {
            let tree_path = gix::path::from_bstr(path.as_bstr());
            if head_tree
                .lookup_entry_by_path(tree_path.as_ref())
                .map_err(|err| {
                    Error::new(ErrorKind::Unexpected, "cannot inspect Git HEAD tree")
                        .with_source(err)
                })?
                .is_some()
            {
                pending.insert(*id);
            }
        }
        if pending.is_empty() {
            return Ok(HashMap::new());
        }

        let mut resource_cache = repo
            .diff_resource_cache(gix::diff::blob::pipeline::Mode::ToGit, Default::default())
            .map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot prepare Git tree diff").with_source(err)
            })?;
        let mut history = HashMap::<PathBuf, FileHistory>::new();
        let mut visited = VisitedPaths::new(files_by_id.len());
        for path in &pending {
            visited.insert(head, *path);
        }
        let mut queue = VecDeque::from([(head, pending)]);
        while let Some((commit_id, paths)) = queue.pop_front() {
            let commit = repo.find_commit(commit_id).map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit").with_source(err)
            })?;
            let parents = commit
                .parent_ids()
                .map(|parent| parent.detach())
                .collect::<Vec<_>>();
            let tree = commit.tree().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot read Git commit tree").with_source(err)
            })?;

            let mut parent_changes = Vec::with_capacity(parents.len());
            for parent in parents {
                let previous_tree = repo
                    .find_commit(parent)
                    .map_err(|err| {
                        Error::new(ErrorKind::Unexpected, "cannot read Git parent commit")
                            .with_source(err)
                    })?
                    .tree()
                    .map_err(|err| {
                        Error::new(ErrorKind::Unexpected, "cannot read Git parent tree")
                            .with_source(err)
                    })?;
                let mut changes = previous_tree.changes().map_err(|err| {
                    Error::new(ErrorKind::Unexpected, "cannot configure Git tree diff")
                        .with_source(err)
                })?;
                // A rename is a removal and an addition for header-history purposes. Avoid the
                // much more expensive similarity analysis and start a new path lifetime instead.
                changes.options(|options| {
                    options.track_rewrites(None);
                });
                let mut path_changes = HashMap::new();
                changes
                    .for_each_to_obtain_tree_with_cache::<Infallible>(
                        &tree,
                        &mut resource_cache,
                        |change| {
                            let (location, path_change) = match change {
                                gix::object::tree::diff::Change::Addition { location, .. } => {
                                    (location, PathChange::Addition)
                                }
                                gix::object::tree::diff::Change::Modification {
                                    location, ..
                                }
                                | gix::object::tree::diff::Change::Deletion { location, .. } => {
                                    (location, PathChange::Modification)
                                }
                                gix::object::tree::diff::Change::Rewrite { .. } => {
                                    unreachable!("rewrite tracking is disabled")
                                }
                            };
                            if let Some(id) = selected.get(location)
                                && paths.contains(id)
                            {
                                path_changes.insert(*id, path_change);
                            }
                            Ok(ControlFlow::Continue(()))
                        },
                    )
                    .map_err(|err| {
                        Error::new(ErrorKind::Unexpected, "cannot compare Git commit trees")
                            .with_source(err)
                    })?;
                parent_changes.push((parent, path_changes));
            }

            let mut routes = HashMap::<gix::ObjectId, HashSet<usize>>::new();
            let mut commit_changes = Vec::new();
            for path in paths {
                // A merge result identical to a parent came from that parent. Following only the
                // first matching parent prevents discarded side-branch changes from leaking in.
                if let Some((parent, _)) = parent_changes
                    .iter()
                    .find(|(_, changes)| !changes.contains_key(&path))
                {
                    routes.entry(*parent).or_default().insert(path);
                    continue;
                }

                let mut existed = false;
                for (parent, changes) in &parent_changes {
                    if changes.get(&path) == Some(&PathChange::Modification) {
                        existed = true;
                        routes.entry(*parent).or_default().insert(path);
                    }
                }
                let change = if existed {
                    PathChange::Modification
                } else {
                    PathChange::Addition
                };
                commit_changes.push((path, change));
            }

            if !commit_changes.is_empty() {
                let year = commit_year(commit.time().map_err(|err| {
                    Error::new(ErrorKind::Unexpected, "cannot read Git commit time")
                        .with_source(err)
                })?)?;
                let author = commit.author().map_err(|err| {
                    Error::new(ErrorKind::Unexpected, "cannot read Git commit author")
                        .with_source(err)
                })?;
                let author = String::from_utf8_lossy(author.name.as_ref()).into_owned();
                for (path, change) in commit_changes {
                    let file = &files_by_id[path];
                    let history = history.entry(file.clone()).or_default();
                    match change {
                        PathChange::Addition => history.record_addition(year, &author),
                        PathChange::Modification => history.record_change(year, &author),
                    }
                }
            }

            for (parent, mut paths) in routes {
                paths.retain(|path| visited.insert(parent, *path));
                if !paths.is_empty() {
                    queue.push_back((parent, paths));
                }
            }
        }
        log::debug!(
            "traversed {} commits for {} Git path histories",
            visited.by_commit.len(),
            files_by_id.len(),
        );
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

    Some(literal_pathspec(prefix.as_bstr()))
}

fn literal_pathspec(path: &BStr) -> BString {
    let mut pattern = BString::from(":(top,literal)");
    pattern.extend_from_slice(path);
    pattern
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
