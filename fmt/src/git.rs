// Copyright 2024 tison <wander4096@gmail.com>
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

use std::collections::hash_map::Entry;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use exn::bail;
use exn::ErrorExt;
use exn::Result;
use exn::ResultExt;
use gix::diff::tree_with_rewrites::Change;
use gix::status::Item;
use gix::Repository;
use walkdir::WalkDir;

use crate::config;
use crate::config::FeatureGate;
use crate::error::Error;

#[derive(Debug, Clone)]
pub struct GitContext {
    pub repo: Option<Repository>,
    pub config: config::Git,
}

pub fn discover(basedir: &Path, config: config::Git) -> Result<GitContext, Error> {
    let feature = resolve_features(&config);

    if feature.is_disable() {
        return Ok(GitContext { repo: None, config });
    }

    match gix::discover(basedir) {
        Ok(repo) => match repo.worktree() {
            None => {
                let message = "bare repository detected";
                if feature.is_auto() {
                    log::info!(config:?; "git config is resolved to disabled; {message}");
                    Ok(GitContext { repo: None, config })
                } else {
                    bail!(Error::new(format!("invalid config: {message}")))
                }
            }
            Some(_) => {
                log::info!("git config is resolved to enabled");
                Ok(GitContext {
                    repo: Some(repo),
                    config,
                })
            }
        },
        Err(err) => {
            if feature.is_auto() {
                log::info!(err:?, config:?; "git config is resolved to disabled");
                Ok(GitContext { repo: None, config })
            } else {
                Err(err
                    .raise()
                    .raise(Error::new("cannot discover git repository with gix")))
            }
        }
    }
}

fn resolve_features(config: &config::Git) -> FeatureGate {
    let features = [config.attrs, config.ignore];
    for feature in features.iter() {
        if feature.is_enable() {
            return FeatureGate::Enable;
        }
    }
    for feature in features.iter() {
        if feature.is_auto() {
            return FeatureGate::Auto;
        }
    }
    FeatureGate::Disable
}

#[derive(Debug)]
pub struct GitFileAttrs {
    pub created_time: gix::date::Time,
    pub modified_time: gix::date::Time,
    pub authors: BTreeSet<String>,
}

pub fn resolve_file_attrs(
    git_context: GitContext,
) -> Result<HashMap<PathBuf, GitFileAttrs>, Error> {
    resolve_file_attrs_of(git_context, None)
}

/// Resolve the Git attributes of the files of interest, given as absolute paths.
///
/// Passing `None` resolves the attributes of every file in the repository, which requires walking
/// the whole history of the repository.
///
/// Otherwise, only the attributes of the given files are resolved, and the traversal stops as soon
/// as the commits that added them are found. Note that a file which has been deleted and added
/// again may then report the most recent addition as its creation, like a renamed file does.
pub fn resolve_file_attrs_of(
    git_context: GitContext,
    interests: Option<&HashSet<PathBuf>>,
) -> Result<HashMap<PathBuf, GitFileAttrs>, Error> {
    if git_context.config.attrs.is_disable() {
        return Ok(HashMap::new());
    }

    let mut repo = match git_context.repo {
        Some(repo) => repo,
        None => return Ok(HashMap::new()),
    };

    let current_username = match repo.committer() {
        Some(Ok(username)) => username.name.to_string(),
        _ => "<unknown>".to_string(),
    };

    let make_error = || Error::new("cannot resolve git file attributes");

    // Traversing the history diffs one commit against another over and over, so most of the trees
    // are decoded again and again. Cache them, or the object database dominates the runtime.
    let object_cache_size = {
        let index = repo.index_or_empty().or_raise(make_error)?;
        repo.compute_object_cache_size_for_tree_diffs(&index)
    };
    repo.object_cache_size_if_unset(object_cache_size);

    let worktree = repo.worktree().expect("worktree cannot be absent");
    let workdir = repo.workdir().expect("workdir cannot be absent");
    let workdir = workdir.canonicalize().or_raise(|| {
        Error::new(format!(
            "cannot resolve absolute path: {}",
            workdir.display()
        ))
    })?;

    let mut excludes = worktree
        .excludes(None)
        .or_raise(|| Error::new("cannot create gix exclude stack"))?;

    let head = repo.head_commit().or_raise(make_error)?;

    // Files of interest whose creation is yet to be found in the history. Files that are not part
    // of HEAD, be they untracked or freshly added, are not in it; they are resolved as committed
    // now while processing the dirty working tree.
    let mut pending = HashSet::new();
    if let Some(interests) = interests {
        let head_tree = head.tree().or_raise(make_error)?;
        for filepath in interests {
            let Ok(rela_path) = filepath.strip_prefix(&workdir) else {
                continue;
            };
            if head_tree
                .lookup_entry_by_path(rela_path)
                .or_raise(make_error)?
                .is_some()
            {
                pending.insert(filepath.clone());
            }
        }
    }

    let mut attrs = if interests.is_some() {
        let mut attrs = HashMap::new();
        let mut additions = vec![];
        let mut ancestors = head.ancestors().all().or_raise(make_error)?;

        // Walk commit by commit for a while: the files of interest may have been added recently,
        // in which case the traversal is over after a handful of commits.
        let mut walked = 0;
        while !pending.is_empty() && walked < SEQUENTIAL_COMMITS {
            let Some(info) = ancestors.next() else { break };
            let info = info.or_raise(make_error)?;
            resolve_commit_attrs(
                &repo,
                &workdir,
                interests,
                &info.id,
                &mut attrs,
                &mut additions,
            )?;
            for filepath in additions.iter() {
                pending.remove(filepath);
            }
            walked += 1;
        }

        if pending.is_empty() {
            log::debug!("stop traversing history since all files of interest are resolved");
        } else {
            // Walking commit by commit is not paying off, so hand the rest to the threads.
            let mut commits = vec![];
            for info in ancestors {
                commits.push(info.or_raise(make_error)?.id);
            }
            let rest =
                resolve_commits_attrs(&repo, &workdir, interests, object_cache_size, &commits)?;
            merge_file_attrs(&mut attrs, rest);
        }

        attrs
    } else {
        resolve_history_attrs(&repo, &workdir, object_cache_size, head)?
    };

    let mut update_attrs = |rela_path: &Path, time: gix::date::Time, author: &str| {
        let filepath = workdir.join(rela_path);
        if interests.is_some_and(|interests| !interests.contains(&filepath)) {
            return;
        }
        update_file_attrs(&mut attrs, filepath, time, author);
    };

    // process dirty working tree
    let index = repo.index_or_empty().or_raise(make_error)?;
    let status_platform = repo.status(gix::progress::Discard).or_raise(make_error)?;
    let status_iter = status_platform.into_iter(None).or_raise(make_error)?;
    let now = gix::date::Time::now_local_or_utc();
    for item in status_iter {
        match item.or_raise(|| Error::new("failed to check git status item"))? {
            Item::IndexWorktree(item) => match item {
                gix::status::index_worktree::Item::Modification { rela_path, .. } => {
                    let rela_path = gix::path::from_bstring(rela_path);
                    update_attrs(&rela_path, now, current_username.as_str());
                }
                gix::status::index_worktree::Item::DirectoryContents { entry, .. } => {
                    if entry.disk_kind.is_some_and(|k| k.is_dir()) {
                        let dirpath = workdir.join(gix::path::from_bstr(&entry.rela_path));
                        if interests.is_some_and(|interests| {
                            !interests
                                .iter()
                                .any(|filepath| filepath.starts_with(&dirpath))
                        }) {
                            log::debug!(dirpath:?; "skip untracked directory without file of interest");
                            continue;
                        }
                        let mut it = WalkDir::new(dirpath).follow_links(false).into_iter();
                        while let Some(entry) = it.next() {
                            let entry =
                                entry.or_raise(|| Error::new("cannot traverse directory"))?;
                            let path = entry.path();
                            let file_type = entry.file_type();
                            if !file_type.is_file() && !file_type.is_dir() {
                                log::debug!(file_type:?; "skip file: {path:?}");
                                continue;
                            }

                            let rela_path = path
                                .strip_prefix(&workdir)
                                .expect("git repository encloses iteration");
                            let mode = Some(if file_type.is_dir() {
                                gix::index::entry::Mode::DIR
                            } else {
                                gix::index::entry::Mode::FILE
                            });
                            let platform = excludes
                                .at_path(rela_path, mode)
                                .or_raise(|| Error::new("cannot check gix exclude"))?;

                            if file_type.is_dir() {
                                if platform.is_excluded() {
                                    let rela =
                                        gix::path::try_into_bstr(rela_path).or_raise(|| {
                                            Error::new("cannot convert path to git path")
                                        })?;

                                    if !index.path_is_directory(rela.as_ref()) {
                                        log::debug!(path:?, rela_path:?; "skip git ignored directory");
                                        it.skip_current_dir();
                                        continue;
                                    }
                                }
                            } else if file_type.is_file() {
                                if platform.is_excluded() {
                                    let rela =
                                        gix::path::try_into_bstr(rela_path).or_raise(|| {
                                            Error::new("cannot convert path to git path")
                                        })?;

                                    if index.entry_by_path(rela.as_ref()).is_none() {
                                        log::debug!(path:?, rela_path:?; "skip git ignored file");
                                        continue;
                                    }
                                }
                                update_attrs(rela_path, now, current_username.as_str());
                            }
                        }
                    } else {
                        let rela_path = gix::path::from_bstring(entry.rela_path);
                        update_attrs(&rela_path, now, current_username.as_str());
                    }
                }
                gix::status::index_worktree::Item::Rewrite { .. } => {
                    unreachable!("rewrite has been disabled")
                }
            },
            Item::TreeIndex(item) => {
                let rela_path = gix::path::from_bstr(item.location());
                update_attrs(&rela_path, now, current_username.as_str());
            }
        }
    }

    Ok(attrs)
}

/// The smallest number of commits worth handing over to a thread of its own.
const COMMITS_PER_THREAD: usize = 512;

/// How many commits to walk one by one before giving up on resolving the files of interest early.
const SEQUENTIAL_COMMITS: usize = 512;

/// Resolve the attributes of every file in the history reachable from `head`.
fn resolve_history_attrs(
    repo: &Repository,
    workdir: &Path,
    object_cache_size: usize,
    head: gix::Commit<'_>,
) -> Result<HashMap<PathBuf, GitFileAttrs>, Error> {
    let make_error = || Error::new("cannot resolve git file attributes");

    let mut commits = vec![];
    for info in head.ancestors().all().or_raise(make_error)? {
        commits.push(info.or_raise(make_error)?.id);
    }

    resolve_commits_attrs(repo, workdir, None, object_cache_size, &commits)
}

/// Fold the changes of the given commits into one set of attributes.
///
/// Commits are diffed against their own parent, so they can be spread over as many threads as the
/// machine affords, each of them folding its share of the history into attributes of its own.
fn resolve_commits_attrs(
    repo: &Repository,
    workdir: &Path,
    interests: Option<&HashSet<PathBuf>>,
    object_cache_size: usize,
    commits: &[gix::ObjectId],
) -> Result<HashMap<PathBuf, GitFileAttrs>, Error> {
    let threads = std::thread::available_parallelism()
        .map(|threads| threads.get())
        .unwrap_or(1)
        .min(commits.len().div_ceil(COMMITS_PER_THREAD))
        .max(1);
    log::debug!(
        "traversing {} commits with {threads} thread(s)",
        commits.len()
    );

    if threads == 1 {
        let mut attrs = HashMap::new();
        let mut additions = vec![];
        for id in commits.iter() {
            resolve_commit_attrs(repo, workdir, interests, id, &mut attrs, &mut additions)?;
        }
        return Ok(attrs);
    }

    // Each thread caches the objects of its own share of the history. Handing out contiguous
    // chunks keeps those caches warm, as neighbouring commits share most of their trees.
    let thread_cache_size = (object_cache_size / threads).max(4 * 1024 * 1024);
    let repo = repo.clone().into_sync();
    let chunk_size = commits.len().div_ceil(threads);

    let mut attrs = HashMap::new();
    let results = std::thread::scope(|scope| {
        let handles = commits
            .chunks(chunk_size)
            .map(|chunk| {
                let repo = &repo;
                scope.spawn(move || -> Result<HashMap<PathBuf, GitFileAttrs>, Error> {
                    let mut repo = repo.to_thread_local();
                    repo.object_cache_size_if_unset(thread_cache_size);

                    let mut attrs = HashMap::new();
                    let mut additions = vec![];
                    for id in chunk.iter() {
                        resolve_commit_attrs(
                            &repo,
                            workdir,
                            interests,
                            id,
                            &mut attrs,
                            &mut additions,
                        )?;
                    }
                    Ok(attrs)
                })
            })
            .collect::<Vec<_>>();

        handles
            .into_iter()
            .map(|handle| handle.join())
            .collect::<Vec<_>>()
    });

    for result in results {
        match result {
            Ok(result) => merge_file_attrs(&mut attrs, result?),
            Err(_) => bail!(Error::new("a thread traversing the history panicked")),
        }
    }

    Ok(attrs)
}

/// Fold the changes a commit brings into `attrs`, and report the files it adds in `additions`.
///
/// Merge commits are skipped, as `git log` does by default: whatever they merge is attributed to
/// the commits of the merged branches.
fn resolve_commit_attrs(
    repo: &Repository,
    workdir: &Path,
    interests: Option<&HashSet<PathBuf>>,
    id: &gix::oid,
    attrs: &mut HashMap<PathBuf, GitFileAttrs>,
    additions: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    let make_error = || Error::new("cannot resolve git file attributes");

    additions.clear();

    let commit = repo.find_commit(id).or_raise(make_error)?;
    let mut parents = commit.parent_ids();
    let parent = parents.next();
    if parents.next().is_some() {
        return Ok(());
    }

    // Diffing a commit against its own parent keeps the diff minimal, since the two trees share
    // every subtree that the commit does not touch.
    let parent_tree = match parent {
        Some(parent) => {
            let parent = repo.find_commit(parent).or_raise(make_error)?;
            Some(parent.tree().or_raise(make_error)?)
        }
        None => None, // the root commit adds all the files it holds
    };
    let this_tree = commit.tree().or_raise(make_error)?;

    let option = {
        let mut option = gix::diff::Options::default();
        option.track_path();
        option
    };
    let changes = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&this_tree), Some(option))
        .or_raise(make_error)?;
    if changes.is_empty() {
        return Ok(());
    }

    let time = commit.time().or_raise(make_error)?;
    let author = commit.author().or_raise(make_error)?.name.to_string();

    for change in changes {
        let (location, entry_mode, added) = match change {
            Change::Addition {
                location,
                entry_mode,
                ..
            } => (location, entry_mode, true),
            Change::Modification {
                location,
                entry_mode,
                ..
            } => (location, entry_mode, false),
            Change::Deletion { .. } => continue, // skip deletion
            Change::Rewrite { .. } => unreachable!("rewrite has been disabled"),
        };

        // only files ever carry a license header; keeping directories would bloat the attributes
        if !entry_mode.is_blob_or_symlink() {
            continue;
        }

        let filepath = workdir.join(gix::path::from_bstring(location));
        if interests.is_some_and(|interests| !interests.contains(&filepath)) {
            continue;
        }
        if added {
            additions.push(filepath.clone());
        }
        update_file_attrs(attrs, filepath, time, &author);
    }

    Ok(())
}

fn update_file_attrs(
    attrs: &mut HashMap<PathBuf, GitFileAttrs>,
    filepath: PathBuf,
    time: gix::date::Time,
    author: &str,
) {
    match attrs.entry(filepath) {
        Entry::Occupied(mut ent) => {
            let attrs: &mut GitFileAttrs = ent.get_mut();
            attrs.created_time = time.min(attrs.created_time);
            attrs.modified_time = time.max(attrs.modified_time);
            attrs.authors.insert(author.to_string());
        }
        Entry::Vacant(ent) => {
            ent.insert(GitFileAttrs {
                created_time: time,
                modified_time: time,
                authors: {
                    let mut authors = BTreeSet::new();
                    authors.insert(author.to_string());
                    authors
                },
            });
        }
    }
}

fn merge_file_attrs(
    attrs: &mut HashMap<PathBuf, GitFileAttrs>,
    other: HashMap<PathBuf, GitFileAttrs>,
) {
    for (filepath, other) in other {
        match attrs.entry(filepath) {
            Entry::Occupied(mut ent) => {
                let attrs: &mut GitFileAttrs = ent.get_mut();
                attrs.created_time = other.created_time.min(attrs.created_time);
                attrs.modified_time = other.modified_time.max(attrs.modified_time);
                attrs.authors.extend(other.authors);
            }
            Entry::Vacant(ent) => {
                ent.insert(other);
            }
        }
    }
}
