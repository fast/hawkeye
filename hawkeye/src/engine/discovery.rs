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
use crate::config::FeatureMode;
use crate::engine::git::Repository;

impl Engine {
    pub(super) fn discover(
        &self,
        repo: Option<&Repository>,
        requested_paths: Option<&[PathBuf]>,
    ) -> Result<Vec<PathBuf>, Error> {
        let started = Instant::now();
        let (files, source) = match requested_paths {
            None => (self.discover_under(&self.root, repo)?, "files.root"),
            Some(paths) => {
                let (directories, direct_files) = self.resolve_requested_paths(paths)?;
                let mut files = direct_files;
                for directory in directories {
                    files.extend(self.discover_under(&directory, repo)?);
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
            "selected {} files through {source} in {:?}",
            files.len(),
            started.elapsed()
        );
        Ok(files)
    }

    fn discover_under(
        &self,
        scan_root: &Path,
        repo: Option<&Repository>,
    ) -> Result<BTreeSet<PathBuf>, Error> {
        if self.git.ignore != FeatureMode::Disable
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
            files.retain(|path| self.selection.matched(path, false).is_whitelist());
            Ok(files)
        } else {
            walk(
                &self.root,
                scan_root,
                &self.selection,
                &self.exclusions,
                self.git.ignore,
            )
        }
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
                    log::warn!(
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
                if self.is_selected_file(&relative) {
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

    fn is_selected_file(&self, path: &Path) -> bool {
        for parent in path.ancestors().skip(1) {
            if !parent.as_os_str().is_empty() && self.selection.matched(parent, true).is_ignore() {
                return false;
            }
        }
        self.selection.matched(path, false).is_whitelist()
    }
}

pub fn compile_patterns(
    root: &Path,
    includes: &[String],
    excludes: &[String],
) -> Result<(Override, Override), Error> {
    let mut builder = OverrideBuilder::new(root);
    if includes.is_empty() {
        builder.add("**").map_err(selection_error)?;
    } else {
        for pattern in includes {
            builder.add(pattern).map_err(selection_error)?;
        }
    }
    builder.add("!.git").map_err(selection_error)?;
    builder.add("!.git/**").map_err(selection_error)?;
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(selection_error)?;
    }
    let selection = builder.build().map_err(selection_error)?;

    let mut builder = OverrideBuilder::new(root);
    builder.add("!.git").map_err(selection_error)?;
    builder.add("!.git/**").map_err(selection_error)?;
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(selection_error)?;
    }
    let exclusions = builder.build().map_err(selection_error)?;
    Ok((selection, exclusions))
}

fn selection_error(err: ignore::Error) -> Error {
    Error::new(
        ErrorKind::ConfigInvalid,
        "invalid files.includes or files.excludes pattern",
    )
    .with_source(err)
}

fn walk(
    root: &Path,
    scan_root: &Path,
    selection: &Override,
    exclusions: &Override,
    git_ignore: FeatureMode,
) -> Result<BTreeSet<PathBuf>, Error> {
    let use_git_ignore = git_ignore != FeatureMode::Disable;
    let mut files = BTreeSet::new();
    let walker = WalkBuilder::new(scan_root)
        .hidden(false)
        .ignore(false)
        .git_ignore(use_git_ignore)
        .git_global(use_git_ignore)
        .git_exclude(use_git_ignore)
        .parents(use_git_ignore)
        .follow_links(false)
        .overrides(exclusions.clone())
        .build();
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

        let relative = path.strip_prefix(root).map_err(|_| {
            Error::new(
                ErrorKind::Unexpected,
                format!(
                    "file walker returned path outside files.root: {}",
                    path.display()
                ),
            )
        })?;

        if selection.matched(relative, false).is_whitelist() {
            files.insert(relative.to_path_buf());
        }
    }
    Ok(files)
}
