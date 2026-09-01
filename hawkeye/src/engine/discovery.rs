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
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use ignore::WalkBuilder;
use ignore::overrides::Override;
use ignore::overrides::OverrideBuilder;

use crate::Engine;
use crate::Error;
use crate::ErrorKind;
use crate::Scope;
use crate::config::FeatureMode;
use crate::engine::git::Repository;

impl Engine {
    pub(super) fn discover_files(
        &self,
        repo: Option<&Repository>,
        scope: Scope<'_>,
    ) -> Result<Vec<PathBuf>, Error> {
        let started = Instant::now();
        let (files, source) = match scope {
            Scope::All => (self.discover_directory(&self.root, repo)?, "files.root"),
            Scope::Paths(paths) => {
                let (directories, direct_files) = self.resolve_requested_paths(paths)?;
                let mut files = direct_files;
                for directory in directories {
                    files.extend(self.discover_directory(&directory, repo)?);
                }
                (files, "the requested paths")
            }
        };

        let files = if let Some(header_path) = &self.header_path {
            let mut selected = Vec::with_capacity(files.len());
            for path in files {
                let absolute_path = self.root.join(&path);
                let is_header =
                    same_file::is_same_file(&absolute_path, header_path).map_err(|err| {
                        Error::new(
                            ErrorKind::Unexpected,
                            format!(
                                "cannot compare {} with header template {}",
                                absolute_path.display(),
                                header_path.display()
                            ),
                        )
                        .with_source(err)
                    })?;
                if !is_header {
                    selected.push(path);
                }
            }
            selected
        } else {
            files.into_iter().collect()
        };
        log::debug!(
            "discovered {} candidate files from {source} in {:?}",
            files.len(),
            started.elapsed()
        );
        Ok(files)
    }

    fn discover_directory(
        &self,
        scan_root: &Path,
        repo: Option<&Repository>,
    ) -> Result<BTreeSet<PathBuf>, Error> {
        let started = Instant::now();
        let (files, backend) = if self.git.ignore != FeatureMode::Disable
            && let Some(repo) = repo
        {
            let prefix = scan_root
                .strip_prefix(&self.root)
                .expect("requested directories are inside files.root");
            let mut files = repo
                .list_files(scan_root)?
                .into_iter()
                .map(|path| prefix.join(path))
                .collect::<BTreeSet<_>>();
            files.retain(|path| self.file_filter.matched(path, false).is_whitelist());
            (files, "Git worktree")
        } else {
            (self.walk_directory(scan_root)?, "filesystem walk")
        };
        log::debug!(
            "discovered {} files under {} via {backend} in {:?}",
            files.len(),
            scan_root.display(),
            started.elapsed()
        );
        Ok(files)
    }

    fn resolve_requested_paths(
        &self,
        paths: &[PathBuf],
    ) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>), Error> {
        let mut directories = BTreeSet::new();
        let mut files = BTreeSet::new();

        for requested in paths {
            let path = if requested.is_absolute() {
                requested.clone()
            } else {
                self.root.join(requested)
            };
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    log::warn!("skipping path that does not exist: {}", requested.display());
                    continue;
                }
                Err(err) => {
                    return Err(Error::new(
                        ErrorKind::Unexpected,
                        format!("cannot inspect requested path {}", requested.display()),
                    )
                    .with_source(err));
                }
            };

            // Keep a final file symlink as the selected path, while resolving parent symlinks and
            // `..` before checking that it remains under files.root.
            let resolved = if metadata.file_type().is_symlink() && path.is_file() {
                let parent = path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .canonicalize()
                    .map_err(|err| {
                        Error::new(
                            ErrorKind::Unexpected,
                            format!("cannot resolve requested path {}", requested.display()),
                        )
                        .with_source(err)
                    })?;
                parent.join(
                    path.file_name()
                        .expect("a file symlink must have a filename"),
                )
            } else {
                path.canonicalize().map_err(|err| {
                    Error::new(
                        ErrorKind::Unexpected,
                        format!("cannot resolve requested path {}", requested.display()),
                    )
                    .with_source(err)
                })?
            };
            let relative = match resolved.strip_prefix(&self.root) {
                Ok(relative) => relative.to_path_buf(),
                Err(_) => {
                    log::debug!(
                        "skipping path outside files.root {}: {}",
                        self.root.display(),
                        requested.display()
                    );
                    continue;
                }
            };

            if metadata.is_dir() {
                directories.insert(resolved);
            } else if metadata.is_file()
                || (metadata.file_type().is_symlink() && resolved.is_file())
            {
                if self.matches_file_filter(&relative) {
                    files.insert(relative);
                }
            } else {
                log::warn!(
                    "skipping path that is not a regular file or directory: {}",
                    requested.display()
                );
            }
        }

        Ok((directories, files))
    }

    fn matches_file_filter(&self, path: &Path) -> bool {
        // An explicitly named file is not walked, so apply exclusions inherited from its parents.
        for parent in path.ancestors().skip(1) {
            if !parent.as_os_str().is_empty() && self.file_filter.matched(parent, true).is_ignore()
            {
                return false;
            }
        }
        self.file_filter.matched(path, false).is_whitelist()
    }

    fn walk_directory(&self, scan_root: &Path) -> Result<BTreeSet<PathBuf>, Error> {
        let use_git_ignore = self.git.ignore != FeatureMode::Disable;
        let mut files = BTreeSet::new();
        let walk_root = if use_git_ignore {
            &self.root
        } else {
            scan_root
        };
        let mut builder = WalkBuilder::new(walk_root);
        builder
            .hidden(false)
            .ignore(false)
            .git_ignore(use_git_ignore)
            .git_global(false)
            .git_exclude(false)
            .parents(false)
            .require_git(false)
            .follow_links(false)
            .overrides(self.walk_filter.clone());
        if walk_root != scan_root {
            let scan_root = scan_root.to_path_buf();
            builder.filter_entry(move |entry| {
                entry.path().starts_with(&scan_root) || scan_root.starts_with(entry.path())
            });
        }
        let walker = builder.build();
        for entry in walker {
            let entry = entry.map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot discover files").with_source(err)
            })?;

            let path = entry.path();
            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() && !(file_type.is_symlink() && path.is_file()) {
                continue;
            }

            let relative = path.strip_prefix(&self.root).map_err(|_| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!(
                        "file walker returned path outside files.root: {}",
                        path.display()
                    ),
                )
            })?;

            if self.file_filter.matched(relative, false).is_whitelist() {
                files.insert(relative.to_path_buf());
            }
        }
        Ok(files)
    }
}

pub fn compile_file_filters(
    root: &Path,
    includes: &[String],
    excludes: &[String],
) -> Result<(Override, Override), Error> {
    let mut builder = OverrideBuilder::new(root);
    if includes.is_empty() {
        builder.add("**").map_err(file_filter_error)?;
    } else {
        for pattern in includes {
            builder.add(pattern).map_err(file_filter_error)?;
        }
    }
    builder.add("!.git").map_err(file_filter_error)?;
    builder.add("!.git/**").map_err(file_filter_error)?;
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(file_filter_error)?;
    }
    let file_filter = builder.build().map_err(file_filter_error)?;

    // Includes cannot be walker overrides because they may prune a directory before a descendant
    // has an opportunity to match. The walker therefore receives exclusions only.
    let mut builder = OverrideBuilder::new(root);
    builder.add("!.git").map_err(file_filter_error)?;
    builder.add("!.git/**").map_err(file_filter_error)?;
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(file_filter_error)?;
    }
    let walk_filter = builder.build().map_err(file_filter_error)?;
    Ok((file_filter, walk_filter))
}

fn file_filter_error(err: ignore::Error) -> Error {
    Error::new(
        ErrorKind::ConfigInvalid,
        "invalid files.includes or files.excludes pattern",
    )
    .with_source(err)
}
