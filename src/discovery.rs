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
use std::path::PathBuf;

use globset::GlobSet;
use ignore::WalkBuilder;

use crate::Error;
use crate::FileSelection;
use crate::Result;
use crate::config::compile_globs;

pub(crate) struct Discovery {
    root: PathBuf,
    include: Option<GlobSet>,
    exclude: GlobSet,
    omitted: BTreeSet<PathBuf>,
    use_gitignore: bool,
}

impl Discovery {
    pub(crate) fn new(
        root: PathBuf,
        selection: &FileSelection,
        omitted: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self> {
        Ok(Self {
            root,
            include: if selection.include().is_empty() {
                None
            } else {
                Some(compile_globs("files.include", selection.include())?)
            },
            exclude: compile_globs("files.exclude", selection.exclude())?,
            omitted: omitted.into_iter().collect(),
            use_gitignore: selection.use_gitignore(),
        })
    }

    pub(crate) fn paths(&self) -> Result<Vec<PathBuf>> {
        let mut builder = WalkBuilder::new(&self.root);
        builder
            .standard_filters(false)
            .hidden(false)
            .parents(false)
            .ignore(false)
            .git_ignore(self.use_gitignore)
            .git_global(self.use_gitignore)
            .git_exclude(self.use_gitignore)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != ".git");

        let mut paths = Vec::new();
        for entry in builder.build() {
            let entry = entry?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.into_path();
            if self.omitted.contains(&path) {
                continue;
            }
            let relative = path.strip_prefix(&self.root).map_err(|_| {
                Error::InvalidConfig(format!(
                    "discovered path {} is outside root {}",
                    path.display(),
                    self.root.display()
                ))
            })?;
            if self
                .include
                .as_ref()
                .is_some_and(|matcher| !matcher.is_match(relative))
                || self.exclude.is_match(relative)
            {
                continue;
            }
            paths.push(path);
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn selection(use_gitignore: bool, include: Vec<String>, exclude: Vec<String>) -> FileSelection {
        FileSelection {
            use_gitignore,
            include,
            exclude,
        }
    }

    #[test]
    fn honors_gitignore_and_explicit_excludes_but_keeps_hidden_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(dir.path().join("ignored.rs"), "ignored").unwrap();
        fs::write(dir.path().join("excluded.rs"), "excluded").unwrap();
        fs::write(dir.path().join("main.rs"), "main").unwrap();
        fs::write(dir.path().join(".hidden.rs"), "hidden").unwrap();

        let discovery = Discovery::new(
            dir.path().to_path_buf(),
            &selection(true, Vec::new(), vec!["excluded.rs".to_owned()]),
            [dir.path().join(".gitignore")],
        )
        .unwrap();
        let relative = discovery
            .paths()
            .unwrap()
            .into_iter()
            .map(|path| path.strip_prefix(dir.path()).unwrap().to_path_buf())
            .collect::<Vec<_>>();

        assert_eq!(
            relative,
            [PathBuf::from(".hidden.rs"), PathBuf::from("main.rs")]
        );
    }

    #[test]
    fn explicit_includes_limit_the_discovery_set() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::create_dir(dir.path().join("tests")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "library").unwrap();
        fs::write(dir.path().join("tests/integration.rs"), "test").unwrap();
        fs::write(dir.path().join("README.md"), "readme").unwrap();

        let discovery = Discovery::new(
            dir.path().to_path_buf(),
            &selection(false, vec!["src/**".to_owned()], Vec::new()),
            [],
        )
        .unwrap();
        let relative = discovery
            .paths()
            .unwrap()
            .into_iter()
            .map(|path| path.strip_prefix(dir.path()).unwrap().to_path_buf())
            .collect::<Vec<_>>();

        assert_eq!(relative, [PathBuf::from("src/lib.rs")]);
    }
}
