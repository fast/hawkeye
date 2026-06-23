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

use std::fmt::Display;
use std::fmt::Formatter;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;

use crate::header::model::HeaderDef;

#[derive(Debug)]
pub struct HeaderParser {
    pub begin_pos: usize,
    /// Some if header exists; None if header does not exist.
    pub end_pos: Option<usize>,
    pub file_content: FileContent,
}

pub fn parse_header(
    mut file_content: FileContent,
    header_def: &HeaderDef,
    keywords: &[String],
) -> HeaderParser {
    let mut line = file_content.next_line();

    // 1. find begin position
    let begin_pos = find_first_position(&mut line, &mut file_content, header_def);

    // 2. has header
    let existing_header = existing_header(&mut line, &mut file_content, header_def, keywords);

    // 3. find end position
    let end_pos = if existing_header {
        // we check if there is a header, if the next line is the blank line of the header
        let mut end = file_content.pos;
        line = file_content.next_line();
        if begin_pos == 0 {
            while line.as_ref().map(|l| l.trim().is_empty()).unwrap_or(false) {
                end = file_content.pos;
                line = file_content.next_line();
            }
        }
        if header_def.end_line.ends_with('\n')
            && line.as_ref().map(|l| l.trim().is_empty()).unwrap_or(false)
        {
            end = file_content.pos;
        }
        Some(end)
    } else {
        None
    };

    HeaderParser {
        begin_pos,
        end_pos,
        file_content,
    }
}

fn find_first_position(
    line: &mut Option<String>,
    file_content: &mut FileContent,
    header_def: &HeaderDef,
) -> usize {
    const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

    let mut begin_pos = 0;

    if let Some(l) = line.as_ref() {
        // skip UTF-8 BOM if exists
        if l.as_bytes().starts_with(&UTF8_BOM) {
            log::debug!("Detected UTF-8 BOM for {file_content}; skip");
            begin_pos = 3;
            file_content.reset_to(3);
        }
    }

    if header_def.skip_line_pattern.is_some() {
        // the format expect to find lines to be skipped
        while line
            .as_ref()
            .map(|l| !header_def.is_skip_line(l))
            .unwrap_or(false)
        {
            begin_pos = file_content.pos;
            *line = file_content.next_line();
        }

        // at least we have found the line to skip, or we are the end of the file
        // this time we are going to skip next lines if they match the skip pattern
        while line
            .as_ref()
            .map(|l| header_def.is_skip_line(l))
            .unwrap_or(false)
        {
            begin_pos = file_content.pos;
            *line = file_content.next_line();
        }

        // After skipping everything we are at the end of the file
        // Header has to be at the file beginning
        if line.is_none() {
            begin_pos = 0;
            file_content.reset();
            *line = file_content.next_line();

            // recheck for UTF-8 BOM
            if let Some(l) = line.as_ref() {
                if l.as_bytes().starts_with(&UTF8_BOM) {
                    begin_pos = 3;
                    file_content.reset_to(3);
                }
            }
        }
    }

    begin_pos
}

fn existing_header(
    line: &mut Option<String>,
    file_content: &mut FileContent,
    header_def: &HeaderDef,
    keywords: &[String],
) -> bool {
    // skip blank lines
    while line.as_ref().map(|l| l.trim().is_empty()).unwrap_or(false) {
        *line = file_content.next_line();
    }

    // check if there is already a header
    let l = match line.as_ref() {
        Some(l) if header_def.is_first_header_line(l) => l,
        _ => return false,
    };

    let mut got_header = false;
    let mut in_place_header = String::new();
    in_place_header.push_str(&l.to_lowercase());

    *line = file_content.next_line();

    // skip blank lines before header text
    if header_def.allow_blank_lines {
        while line.as_ref().map(|l| l.trim().is_empty()).unwrap_or(false) {
            *line = file_content.next_line();
        }
    }

    // first header detected line & potential blank lines have been detected
    // following lines should be header lines
    if let Some(l) = line.as_ref() {
        let before = {
            let mut before = header_def.before_each_line.trim_end();
            if before.is_empty() && !header_def.multiple_lines {
                before = header_def.before_each_line.as_str();
            }
            before
        };

        let found_end = {
            let mut found_end = false;
            if (header_def.multiple_lines && header_def.is_last_header_line(l))
                || l.trim().is_empty()
            {
                in_place_header.push_str(&l.to_lowercase());
                found_end = true;
            } else {
                loop {
                    match line.as_ref() {
                        Some(l) if l.starts_with(before) => {
                            in_place_header.push_str(&l.to_lowercase());
                            if header_def.multiple_lines && header_def.is_last_header_line(l) {
                                found_end = true;
                                break;
                            }
                        }
                        _ => break,
                    }
                    *line = file_content.next_line();
                }

                if line.as_ref().map(|l| l.trim().is_empty()).unwrap_or(true) {
                    found_end = true;
                }
            }
            found_end
        };

        // skip blank lines after header text
        if header_def.multiple_lines && header_def.allow_blank_lines && !found_end {
            loop {
                if !line.as_ref().map(|l| l.trim().is_empty()).unwrap_or(false) {
                    break;
                }
                *line = file_content.next_line();
            }
            file_content.rewind();
        } else if !header_def.multiple_lines && !found_end {
            file_content.rewind();
        }

        if !header_def.multiple_lines {
            // keep track of the position for headers where the end line is the same as the
            // before each line
            let pos = file_content.pos;
            // check if the line is the end line
            while line
                .as_ref()
                .map(|l| {
                    !header_def.is_last_header_line(l)
                        && (header_def.allow_blank_lines || !l.trim().is_empty())
                        && l.starts_with(before)
                })
                .unwrap_or(false)
            {
                *line = file_content.next_line();
            }
            if line.is_none() {
                file_content.reset_to(pos);
            }
        } else if line.is_some() {
            // we could end up there if we still have some lines, but not matching "before".
            // This can be the last line in a multi line header
            let pos = file_content.pos;
            *line = file_content.next_line();
            if line
                .as_ref()
                .map(|l| !header_def.is_last_header_line(l))
                .unwrap_or(true)
            {
                file_content.reset_to(pos);
            }
        }

        got_header = true;
        for keyword in keywords {
            if !in_place_header.contains(keyword) {
                got_header = false;
                break;
            }
        }
    }
    // else - we detected previously a one line comment block that matches the header
    // detection it is not a header it is a comment
    got_header
}

#[derive(Debug)]
pub struct FileContent {
    pos: usize,
    old_pos: usize,
    content: String,
    filepath: String,
}

impl Display for FileContent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.filepath)
    }
}

impl FileContent {
    pub fn new(file: &Path) -> std::io::Result<Self> {
        Ok(Self {
            pos: 0,
            old_pos: 0,
            content: {
                let mut content = String::new();
                let mut reader = File::open(file).map(BufReader::new)?;
                let mut buf = String::new();
                let mut n = reader.read_line(&mut buf)?;
                while n > 0 {
                    if buf.ends_with('\n') {
                        buf.pop();
                        if buf.ends_with('\r') {
                            buf.pop();
                        }
                        content.push_str(&buf);
                        content.push('\n');
                    } else {
                        content.push_str(&buf);
                    }
                    buf.clear();
                    n = reader.read_line(&mut buf)?;
                }
                content
            },
            filepath: file.to_string_lossy().to_string(),
        })
    }

    /// Build a `FileContent` over an already line-normalized in-memory buffer. Used to
    /// re-probe the current content with alternative header styles without touching disk.
    pub fn from_content(content: String, filepath: String) -> Self {
        Self {
            pos: 0,
            old_pos: 0,
            content,
            filepath,
        }
    }

    pub fn reset_to(&mut self, pos: usize) {
        self.old_pos = pos;
        self.pos = pos;
    }

    pub fn reset(&mut self) {
        self.reset_to(0);
    }

    pub fn rewind(&mut self) {
        self.pos = self.old_pos;
    }

    pub fn end_reached(&self) -> bool {
        self.pos >= self.content.len()
    }

    pub fn next_line(&mut self) -> Option<String> {
        if self.end_reached() {
            return None;
        }

        let lf = self.content[self.pos..].find('\n').map(|i| i + self.pos);
        let eol = lf.unwrap_or(self.content.len());
        let result = self.content[self.pos..eol].to_string();

        self.old_pos = self.pos;
        self.pos = if let Some(lf) = lf {
            lf + 1
        } else {
            self.content.len()
        };

        Some(result)
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn insert(&mut self, index: usize, s: &str) {
        self.content.insert_str(index, s);
    }

    pub fn delete(&mut self, start: usize, end: usize) {
        self.content.drain(start..end);
    }
}
