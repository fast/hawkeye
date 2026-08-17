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
use crate::ErrorKind;

pub(crate) fn validate_source(path: &Path, expected: &[u8]) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot read metadata for {}", path.display()),
        )
        .with_source(source)
    })?;
    if metadata.file_type().is_symlink() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!("refusing to replace symbolic link {}", path.display()),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("refusing to replace hard-linked file {}", path.display()),
            ));
        }
    }

    let current = fs::read(path).map_err(|source| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot reread {}", path.display()),
        )
        .with_source(source)
    })?;
    if current != expected {
        return Err(Error::new(
            ErrorKind::StalePlan,
            format!("{} changed after it was analyzed", path.display()),
        ));
    }
    Ok(())
}

pub(crate) fn write_atomic(path: &Path, expected: &[u8], updated: &[u8]) -> Result<(), Error> {
    validate_source(path, expected)?;
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot read metadata for {}", path.display()),
        )
        .with_source(source)
    })?;

    let parent = path.parent().ok_or_else(|| {
        Error::new(
            ErrorKind::Unexpected,
            format!("source path has no parent: {}", path.display()),
        )
    })?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot create temporary file for {}", path.display()),
        )
        .with_source(source)
    })?;
    temporary
        .as_file_mut()
        .set_permissions(metadata.permissions())
        .map_err(|source| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot set permissions for {}", path.display()),
            )
            .with_source(source)
        })?;
    temporary.write_all(updated).map_err(|source| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot write temporary file for {}", path.display()),
        )
        .with_source(source)
    })?;
    temporary.flush().map_err(|source| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot flush temporary file for {}", path.display()),
        )
        .with_source(source)
    })?;
    temporary.as_file().sync_all().map_err(|source| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot sync temporary file for {}", path.display()),
        )
        .with_source(source)
    })?;
    temporary.persist(path).map_err(|error| {
        Error::new(
            ErrorKind::Unexpected,
            format!("cannot replace {}", path.display()),
        )
        .with_source(error.error)
    })?;
    Ok(())
}
