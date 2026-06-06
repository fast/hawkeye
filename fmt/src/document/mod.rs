// Copyright 2024 tison <wander4096@gmail.com>
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
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::BufRead;
use std::path::Path;
use std::path::PathBuf;

use exn::ErrorExt;
use exn::Result;
use exn::ResultExt;
use minijinja::context;
use minijinja::Environment;
use serde::Deserialize;
use serde::Serialize;

use crate::error::Error;
use crate::header::matcher::HeaderMatcher;
use crate::header::model::HeaderDef;
use crate::header::parser::parse_header;
use crate::header::parser::FileContent;
use crate::header::parser::HeaderParser;

pub mod factory;
pub mod model;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attributes {
    pub filename: Option<String>,
    pub disk_file_created_year: Option<i16>,
    pub git_file_created_year: Option<i16>,
    pub git_file_modified_year: Option<i16>,
    pub git_authors: BTreeSet<String>,
}

#[derive(Debug)]
pub struct Document {
    pub filepath: PathBuf,

    header_def: HeaderDef,
    props: HashMap<String, String>,
    attrs: Attributes,
    parser: HeaderParser,
}

impl Document {
    pub fn new(
        filepath: PathBuf,
        header_def: HeaderDef,
        keywords: &[String],
        props: HashMap<String, String>,
        attrs: Attributes,
    ) -> Result<Option<Self>, Error> {
        match FileContent::new(&filepath) {
            Ok(content) => Ok(Some(Self {
                parser: parse_header(content, &header_def, keywords),
                filepath,
                header_def,
                props,
                attrs,
            })),
            Err(err) => {
                if matches!(err.kind(), std::io::ErrorKind::InvalidData) {
                    log::debug!("skip non-textual file: {}", filepath.display());
                    Ok(None)
                } else {
                    Err(err.raise().raise(Error::new(format!(
                        "cannot create document: {}",
                        filepath.display()
                    ))))
                }
            }
        }
    }

    pub fn is_unsupported(&self) -> bool {
        self.header_def.name.eq_ignore_ascii_case("unknown")
    }

    /// Detected but not necessarily a valid header
    pub fn header_detected(&self) -> bool {
        self.parser.end_pos.is_some()
    }

    /// Detected and valid header
    pub fn header_matched(
        &self,
        header: &HeaderMatcher,
        strict_check: bool,
    ) -> Result<bool, Error> {
        if strict_check {
            let file_header = {
                let mut lines = self.read_file_first_lines(header)?.join("\n");
                lines.push_str("\n\n");
                lines.replace(" *\r?\n", "\n")
            };
            let expected_header = {
                let raw_header = header.build_for_definition(&self.header_def);
                let resolved_header = self.merge_properties(&raw_header)?;
                resolved_header.replace(" *\r?\n", "\n")
            };
            Ok(file_header.contains(expected_header.as_str()))
        } else {
            let file_header = self.read_file_header_on_one_line(header)?;
            let expected_header = self.merge_properties(header.header_content_one_line())?;
            Ok(file_header.contains(expected_header.as_str()))
        }
    }

    #[track_caller]
    fn read_file_first_lines(&self, header: &HeaderMatcher) -> Result<Vec<String>, Error> {
        let make_error = || Error::new("cannot read file first line");
        let file = File::open(&self.filepath).or_raise(make_error)?;
        std::io::BufReader::new(file)
            .lines()
            .take(header.header_content_lines_count() + 10)
            .collect::<std::io::Result<Vec<_>>>()
            .or_raise(make_error)
    }

    #[track_caller]
    fn read_file_header_on_one_line(&self, header: &HeaderMatcher) -> Result<String, Error> {
        let first_lines = self.read_file_first_lines(header)?;
        let file_header = first_lines
            .join("")
            .trim()
            .replace(self.header_def.first_line.trim(), "")
            .replace(self.header_def.end_line.trim(), "")
            .replace(self.header_def.before_each_line.trim(), "")
            .replace(self.header_def.after_each_line.trim(), "")
            .split_whitespace()
            .collect();
        Ok(file_header)
    }

    pub fn update_header(&mut self, header: &HeaderMatcher) -> Result<(), Error> {
        let header_str = header.build_for_definition(&self.header_def);
        let header_str = self.merge_properties(&header_str)?;
        let begin_pos = self.parser.begin_pos;
        self.parser
            .file_content
            .insert(begin_pos, header_str.as_str());
        Ok(())
    }

    pub fn remove_header(&mut self) {
        if let Some(end_pos) = self.parser.end_pos {
            self.parser
                .file_content
                .delete(self.parser.begin_pos, end_pos);
        }
    }

    pub fn save(&mut self) -> Result<(), Error> {
        let filepath = self.filepath.as_path();
        fs::write(filepath, self.parser.file_content.content())
            .or_raise(|| Error::new(format!("cannot save document {}", filepath.display())))
    }

    pub fn save_to(&mut self, filepath: impl AsRef<Path>) -> Result<(), Error> {
        let filepath = filepath.as_ref();
        fs::write(filepath, self.parser.file_content.content())
            .or_raise(|| Error::new(format!("cannot save document {}", filepath.display())))
    }

    pub(crate) fn merge_properties(&self, s: &str) -> Result<String, Error> {
        let mut env = Environment::new();
        env.add_template("template", s)
            .or_raise(|| Error::new("malformed template"))?;

        let tmpl = env.get_template("template").expect("template must exist");
        let mut result = tmpl
            .render(context! {
                props => &self.props,
                attrs => &self.attrs,
            })
            .or_raise(|| Error::new("cannot render template"))?;
        result.push('\n');
        Ok(result)
    }
}
