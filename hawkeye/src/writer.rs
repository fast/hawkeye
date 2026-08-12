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
use std::io::Write;
use std::path::Path;

use tempfile::NamedTempFile;

use crate::Error;
use crate::Result;

pub(crate) fn validate_source(path: &Path, expected: &[u8]) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| Error::io("read metadata for", path, source))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::Symlink(path.to_path_buf()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(Error::HardLink(path.to_path_buf()));
        }
    }

    let current = fs::read(path).map_err(|source| Error::io("reread", path, source))?;
    if current != expected {
        return Err(Error::StaleFile(path.to_path_buf()));
    }
    Ok(())
}

pub(crate) fn write_atomic(path: &Path, expected: &[u8], updated: &[u8]) -> Result<()> {
    validate_source(path, expected)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| Error::io("read metadata for", path, source))?;

    let parent = path.parent().ok_or_else(|| {
        Error::InvalidConfig(format!("source path has no parent: {}", path.display()))
    })?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|source| Error::io("create temporary file for", path, source))?;
    temporary
        .as_file_mut()
        .set_permissions(metadata.permissions())
        .map_err(|source| Error::io("set permissions for", path, source))?;
    temporary
        .write_all(updated)
        .map_err(|source| Error::io("write temporary file for", path, source))?;
    temporary
        .flush()
        .map_err(|source| Error::io("flush temporary file for", path, source))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| Error::io("sync temporary file for", path, source))?;
    temporary
        .persist(path)
        .map_err(|error| Error::io("replace", path, error.error))?;
    Ok(())
}
