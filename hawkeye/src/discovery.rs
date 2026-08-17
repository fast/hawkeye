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

use crate::Error;
use crate::ErrorKind;
use crate::ResolvedConfig;
use crate::config::FeatureMode;
use crate::git::GitRepo;

pub(crate) fn discover(
    config: &ResolvedConfig,
    repo: Option<&GitRepo>,
) -> Result<Vec<PathBuf>, Error> {
    let started = Instant::now();
    let selection = build_selection(config)?;
    let mut files = BTreeSet::new();

    if config.git().ignore != FeatureMode::Disable
        && let Some(repo) = repo
    {
        for path in repo.list_files(config.root())? {
            let relative = path
                .strip_prefix(config.root())
                .expect("Git discovery only returns paths inside files.root");
            if selection.matched(relative, false).is_whitelist() {
                files.insert(path);
            }
        }
        log::debug!(
            "selected {} files through the Git index in {:?}",
            files.len(),
            started.elapsed()
        );
    } else {
        let exclusions = build_exclusions(config)?;
        walk(
            config.root(),
            &selection,
            &exclusions,
            config.git().ignore,
            &mut files,
        )?;
        log::debug!(
            "selected {} files through a filesystem walk in {:?}",
            files.len(),
            started.elapsed()
        );
    }

    if let Some(header_path) = config.header_path() {
        files.remove(header_path);
    }
    Ok(files.into_iter().collect())
}

fn build_selection(config: &ResolvedConfig) -> Result<Override, Error> {
    let mut builder = OverrideBuilder::new(config.root());
    if config.includes().is_empty() {
        builder.add("**").map_err(selection_error)?;
    } else {
        for pattern in config.includes() {
            builder.add(pattern).map_err(selection_error)?;
        }
    }
    builder.add("!.git").map_err(selection_error)?;
    builder.add("!.git/**").map_err(selection_error)?;
    for pattern in config.excludes() {
        builder
            .add(&format!("!{pattern}"))
            .map_err(selection_error)?;
    }
    builder.build().map_err(selection_error)
}

fn build_exclusions(config: &ResolvedConfig) -> Result<Override, Error> {
    let mut builder = OverrideBuilder::new(config.root());
    builder.add("!.git").map_err(selection_error)?;
    builder.add("!.git/**").map_err(selection_error)?;
    for pattern in config.excludes() {
        builder
            .add(&format!("!{pattern}"))
            .map_err(selection_error)?;
    }
    builder.build().map_err(selection_error)
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
        let entry = entry.map_err(|source| {
            let kind = if source.is_io() {
                ErrorKind::Io
            } else {
                ErrorKind::Unexpected
            };
            Error::new(kind, "cannot discover files").with_source(source)
        })?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
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
