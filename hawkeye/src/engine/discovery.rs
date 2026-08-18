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
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use ignore::WalkBuilder;
use ignore::overrides::Override;
use ignore::overrides::OverrideBuilder;

use super::Engine;
use super::git::Repository;
use crate::Error;
use crate::ErrorKind;
use crate::config::FeatureMode;

impl Engine {
    pub(super) fn discover(&self, repo: Option<&Repository>) -> Result<Vec<PathBuf>, Error> {
        let started = Instant::now();
        let (files, source) = if self.git.ignore != FeatureMode::Disable
            && let Some(repo) = repo
        {
            let files = repo
                .list_files(&self.root)?
                .into_iter()
                .filter(|path| self.selection.matched(path, false).is_whitelist())
                .collect::<BTreeSet<_>>();
            (files, "the Git index")
        } else {
            let files = walk(
                &self.root,
                &self.selection,
                &self.exclusions,
                self.git.ignore,
            )?;
            (files, "a filesystem walk")
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
    selection: &Override,
    exclusions: &Override,
    git_ignore: FeatureMode,
) -> Result<BTreeSet<PathBuf>, Error> {
    let use_git_ignore = git_ignore != FeatureMode::Disable;
    let mut files = BTreeSet::new();
    let walker = WalkBuilder::new(root)
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
