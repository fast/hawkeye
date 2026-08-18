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

mod history;

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use gix::bstr::BStr;
use gix::bstr::BString;
use gix::bstr::ByteSlice;

pub use self::history::FileHistory;
use crate::Error;
use crate::ErrorKind;

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
        let relative_root = self.relative_scan_root(scan_root)?;
        let prefix = path_prefix(relative_root);
        let index = self.inner.index_or_empty().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot read Git index").with_source(err)
        })?;
        let mut files = BTreeSet::new();
        if let Some(entries) = index.prefixed_entries(prefix.as_bstr()) {
            for entry in entries {
                self.insert_file(entry.path(&index), scan_root, &mut files)?;
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
            self.insert_file(item.entry.rela_path.as_ref(), scan_root, &mut files)?;
        }
        Ok(files)
    }

    pub fn is_shallow(&self) -> bool {
        self.inner.is_shallow()
    }

    fn insert_file(
        &self,
        repository_path: &BStr,
        scan_root: &Path,
        files: &mut BTreeSet<PathBuf>,
    ) -> Result<(), Error> {
        let path = self.root.join(gix::path::from_bstr(repository_path));
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read metadata for {}", path.display()),
                )
                .with_source(err));
            }
        };
        let file_type = metadata.file_type();
        if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
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
            files.insert(relative.to_path_buf());
        }
        Ok(())
    }

    fn relative_scan_root<'a>(&self, scan_root: &'a Path) -> Result<&'a Path, Error> {
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

    let mut pattern = BString::from(":(top,literal)");
    pattern.extend_from_slice(&prefix);
    Some(pattern)
}

fn encode_path(path: &Path) -> BString {
    gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path)).into_owned()
}
