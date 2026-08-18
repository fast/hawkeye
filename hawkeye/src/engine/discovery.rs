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
use super::git::GitRepo;
use crate::Error;
use crate::ErrorKind;
use crate::config::FeatureMode;

impl Engine {
    pub(super) fn discover(&self, repo: Option<&GitRepo>) -> Result<Vec<PathBuf>, Error> {
        let started = Instant::now();
        let mut files = BTreeSet::new();

        if self.git.ignore != FeatureMode::Disable
            && let Some(repo) = repo
        {
            for path in repo.list_files(&self.root)? {
                let relative = path
                    .strip_prefix(&self.root)
                    .expect("Git discovery only returns paths inside files.root");
                if self.selection.matched(relative, false).is_whitelist() {
                    files.insert(path);
                }
            }
            log::debug!(
                "selected {} files through the Git index in {:?}",
                files.len(),
                started.elapsed()
            );
        } else {
            walk(
                &self.root,
                &self.selection,
                &self.exclusions,
                self.git.ignore,
                &mut files,
            )?;
            log::debug!(
                "selected {} files through a filesystem walk in {:?}",
                files.len(),
                started.elapsed()
            );
        }

        if let Some(header_path) = &self.header_path {
            files.retain(|path| {
                path != header_path
                    && (!path.is_symlink()
                        || path
                            .canonicalize()
                            .is_ok_and(|target| target != *header_path))
            });
        }
        Ok(files.into_iter().collect())
    }
}

pub(super) fn compile_patterns(
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

fn selection_error(source: ignore::Error) -> Error {
    Error::new(
        ErrorKind::ConfigInvalid,
        "invalid files.includes or files.excludes pattern",
    )
    .with_source(source)
}

fn walk(
    root: &Path,
    selection: &Override,
    exclusions: &Override,
    git_ignore: FeatureMode,
    files: &mut BTreeSet<PathBuf>,
) -> Result<(), Error> {
    let use_git_ignore = git_ignore != FeatureMode::Disable;
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
        if !entry
            .file_type()
            .is_some_and(|kind| kind.is_file() || (kind.is_symlink() && path.is_file()))
        {
            continue;
        }
        let path = entry.into_path();
        let relative = path
            .strip_prefix(root)
            .expect("walker only yields paths inside files.root");
        if selection.matched(relative, false).is_whitelist() {
            files.insert(path);
        }
    }
    Ok(())
}
