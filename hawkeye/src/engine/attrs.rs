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
use std::path::Path;
use std::time::SystemTime;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use super::git::GitFileHistory;
use crate::Error;
use crate::ErrorKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileAttrs {
    pub filename: String,
    pub disk_file_created_year: Option<i16>,
    pub disk_file_modified_year: Option<i16>,
    pub git_file_created_year: Option<i16>,
    pub git_file_modified_year: Option<i16>,
    pub git_authors: Vec<String>,
}

impl FileAttrs {
    pub fn new(path: &Path, git: Option<&GitFileHistory>) -> Result<Self, Error> {
        let metadata = fs::metadata(path).map_err(|err| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot read metadata for {}", path.display()),
            )
            .with_source(err)
        })?;
        Ok(Self {
            filename: path
                .file_name()
                .expect("discovery only returns files")
                .to_string_lossy()
                .into_owned(),
            disk_file_created_year: metadata.created().ok().and_then(utc_year),
            disk_file_modified_year: metadata.modified().ok().and_then(utc_year),
            git_file_created_year: git.and_then(|history| history.created_year),
            git_file_modified_year: git.and_then(|history| history.modified_year),
            git_authors: git
                .map(|history| history.authors.iter().cloned().collect())
                .unwrap_or_default(),
        })
    }
}

fn utc_year(time: SystemTime) -> Option<i16> {
    Timestamp::try_from(time)
        .ok()
        .map(|timestamp| timestamp.to_zoned(TimeZone::UTC).year())
}
