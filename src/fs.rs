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

use std::fs;
use std::fs::Permissions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use tempfile::NamedTempFile;

use crate::Error;
use crate::Result;

pub(crate) struct PreparedWrite {
    path: PathBuf,
    content: String,
    permissions: Permissions,
}

impl PreparedWrite {
    pub(crate) fn prepare(path: &Path, original: &str, content: String) -> Result<Self> {
        let link_metadata = fs::symlink_metadata(path)
            .map_err(|source| io_error("read metadata for", path, source))?;
        if link_metadata.file_type().is_symlink() {
            return Err(Error::Symlink(path.to_path_buf()));
        }

        let current = fs::read(path).map_err(|source| io_error("read", path, source))?;
        if current != original.as_bytes() {
            return Err(Error::StaleFile(path.to_path_buf()));
        }

        Ok(Self {
            path: path.to_path_buf(),
            content,
            permissions: link_metadata.permissions(),
        })
    }

    pub(crate) fn commit(self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = NamedTempFile::new_in(parent)
            .map_err(|source| io_error("create temporary file beside", &self.path, source))?;
        temporary
            .as_file_mut()
            .write_all(self.content.as_bytes())
            .map_err(|source| io_error("write temporary replacement for", &self.path, source))?;
        temporary
            .as_file()
            .set_permissions(self.permissions)
            .map_err(|source| io_error("preserve permissions for", &self.path, source))?;
        temporary.as_file_mut().sync_all().map_err(|source| {
            io_error("synchronize temporary replacement for", &self.path, source)
        })?;

        let persisted = temporary
            .persist(&self.path)
            .map_err(|error| io_error("atomically replace", &self.path, error.error))?;
        persisted
            .sync_all()
            .map_err(|source| io_error("synchronize", &self.path, source))?;

        sync_parent(parent);
        Ok(())
    }
}

pub(crate) fn io_error(
    operation: &'static str,
    path: impl Into<PathBuf>,
    source: std::io::Error,
) -> Error {
    Error::Io {
        operation,
        path: path.into(),
        source,
    }
}

#[cfg(unix)]
fn sync_parent(parent: &Path) {
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn refuses_to_replace_a_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.rs");
        let link = directory.path().join("link.rs");
        fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = PreparedWrite::prepare(&link, "old", "new".to_owned());
        assert!(matches!(result, Err(Error::Symlink(path)) if path == link));
        assert_eq!(fs::read_to_string(target).unwrap(), "old");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replacement_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.rs");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o640)).unwrap();

        PreparedWrite::prepare(&path, "old", "new".to_owned())
            .unwrap()
            .commit()
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
