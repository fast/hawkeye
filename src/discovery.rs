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
use std::process::Command;

use ignore::WalkBuilder;
use ignore::overrides::Override;
use ignore::overrides::OverrideBuilder;

use crate::Error;
use crate::ResolvedConfig;
use crate::Result;
use crate::config::FeatureMode;

pub(crate) fn discover(config: &ResolvedConfig, targets: &[PathBuf]) -> Result<Vec<PathBuf>> {
    require_git_if_requested(config.root(), config.git().ignore())?;
    let selection = build_selection(config)?;
    let exclusions = build_exclusions(config)?;
    let mut files = BTreeSet::new();

    if targets.is_empty() {
        walk(
            config.root(),
            config.root(),
            &selection,
            &exclusions,
            config.git().ignore(),
            &mut files,
        )?;
    } else {
        let cwd = std::env::current_dir()
            .map_err(|source| Error::io("read current directory for", config.root(), source))?;
        for target in targets {
            let target = if target.is_absolute() {
                target.clone()
            } else {
                cwd.join(target)
            };
            let metadata = std::fs::symlink_metadata(&target)
                .map_err(|source| Error::io("read explicit target", &target, source))?;
            if metadata.file_type().is_symlink() {
                return Err(Error::Symlink(target));
            }
            let target = target
                .canonicalize()
                .map_err(|source| Error::io("resolve explicit target", &target, source))?;
            if !target.starts_with(config.root()) {
                return Err(Error::InvalidTarget(format!(
                    "{} is outside files.root {}",
                    target.display(),
                    config.root().display()
                )));
            }
            if target.is_file() {
                let relative = target
                    .strip_prefix(config.root())
                    .expect("target was checked to be inside root");
                if selection.matched(relative, false).is_whitelist() {
                    files.insert(target);
                }
            } else if target.is_dir() {
                walk(
                    &target,
                    config.root(),
                    &selection,
                    &exclusions,
                    config.git().ignore(),
                    &mut files,
                )?;
            } else {
                return Err(Error::InvalidTarget(format!(
                    "{} is neither a regular file nor a directory",
                    target.display()
                )));
            }
        }
    }

    Ok(files.into_iter().collect())
}

fn build_selection(config: &ResolvedConfig) -> Result<Override> {
    let mut builder = OverrideBuilder::new(config.root());
    if config.includes().is_empty() {
        builder.add("**")?;
    } else {
        for pattern in config.includes() {
            builder.add(pattern)?;
        }
    }
    builder.add("!.git")?;
    builder.add("!.git/**")?;
    for pattern in config.excludes() {
        builder.add(&format!("!{pattern}"))?;
    }
    builder.build().map_err(Into::into)
}

fn build_exclusions(config: &ResolvedConfig) -> Result<Override> {
    let mut builder = OverrideBuilder::new(config.root());
    builder.add("!.git")?;
    builder.add("!.git/**")?;
    for pattern in config.excludes() {
        builder.add(&format!("!{pattern}"))?;
    }
    builder.build().map_err(Into::into)
}

fn walk(
    start: &Path,
    root: &Path,
    selection: &Override,
    exclusions: &Override,
    git_ignore: FeatureMode,
    files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let use_git_ignore = git_ignore != FeatureMode::Disable;
    let walker = WalkBuilder::new(start)
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
        let entry = entry?;
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

fn require_git_if_requested(root: &Path, mode: FeatureMode) -> Result<()> {
    if mode != FeatureMode::Enable {
        return Ok(());
    }
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| Error::Git(format!("cannot start Git: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}
