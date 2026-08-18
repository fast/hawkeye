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

use std::ops::Range;

use super::lines;
use crate::config::StyleConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Style {
    Line {
        prefix: String,
        suffix: String,
        pad_lines: bool,
    },
    Block {
        start: String,
        prefix: String,
        suffix: String,
        end: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleMatch {
    pub range: Range<usize>,
    pub body: String,
}

impl Style {
    pub fn new(config: StyleConfig) -> Self {
        match config {
            StyleConfig::Line {
                prefix,
                suffix,
                pad_lines,
            } => Self::Line {
                prefix,
                suffix,
                pad_lines,
            },
            StyleConfig::Block {
                start,
                prefix,
                suffix,
                end,
            } => Self::Block {
                start,
                prefix,
                suffix,
                end,
            },
        }
    }

    pub fn render(&self, body: &str, eol: &str) -> String {
        let mut output = String::new();
        match self {
            Self::Line {
                prefix,
                suffix,
                pad_lines,
            } => {
                let lines = body.split('\n');
                let width = if *pad_lines {
                    lines
                        .clone()
                        .map(|line| line.chars().count())
                        .max()
                        .unwrap_or(0)
                } else {
                    0
                };
                for (index, line) in lines.enumerate() {
                    if index > 0 {
                        output.push_str(eol);
                    }
                    let line_start = output.len();
                    output.push_str(prefix);
                    output.push_str(line);
                    if *pad_lines {
                        output.extend(std::iter::repeat_n(
                            ' ',
                            width.saturating_sub(line.chars().count()),
                        ));
                    }
                    output.push_str(suffix);
                    if suffix.is_empty() {
                        let len = output[line_start..].trim_end_matches([' ', '\t']).len();
                        output.truncate(line_start + len);
                    }
                }
            }
            Self::Block {
                start,
                prefix,
                suffix,
                end,
            } => {
                output.push_str(start);
                for line in body.split('\n') {
                    output.push_str(eol);
                    let line_start = output.len();
                    output.push_str(prefix);
                    output.push_str(line);
                    output.push_str(suffix);
                    if suffix.is_empty() {
                        let len = output[line_start..].trim_end_matches([' ', '\t']).len();
                        output.truncate(line_start + len);
                    }
                }
                output.push_str(eol);
                output.push_str(end);
            }
        }
        output
    }

    pub fn parse(&self, input: &str, start: usize) -> Option<StyleMatch> {
        match self {
            Self::Line {
                prefix,
                suffix,
                pad_lines,
            } => parse_line_style(input, start, prefix, suffix, *pad_lines),
            Self::Block {
                start: opening,
                prefix,
                suffix,
                end: closing,
            } => parse_block_style(input, start, opening, prefix, suffix, closing),
        }
    }
}

fn parse_line_style(
    input: &str,
    start: usize,
    prefix: &str,
    suffix: &str,
    pad_lines: bool,
) -> Option<StyleMatch> {
    let mut end = start;
    let mut body = Vec::new();
    for (line, raw_range) in lines::iter(input, start) {
        let Some(content) = strip_affixes(line, prefix, suffix, pad_lines) else {
            break;
        };
        body.push(content);
        end = raw_range.start + line.len();
    }
    if body.is_empty() {
        None
    } else {
        Some(StyleMatch {
            range: start..end,
            body: body.join("\n"),
        })
    }
}

fn parse_block_style(
    input: &str,
    start: usize,
    opening: &str,
    prefix: &str,
    suffix: &str,
    closing: &str,
) -> Option<StyleMatch> {
    let mut lines = lines::iter(input, start);
    let (first, _) = lines.next()?;
    if first != opening {
        return None;
    }

    let mut body = Vec::new();
    for (content, raw_range) in lines {
        if content == closing {
            let end = raw_range.start + content.len();
            return Some(StyleMatch {
                range: start..end,
                body: body.join("\n"),
            });
        }
        body.push(strip_affixes(content, prefix, suffix, false)?);
    }
    None
}

fn strip_affixes(line: &str, prefix: &str, suffix: &str, pad_lines: bool) -> Option<String> {
    let prefix_without_space = prefix.trim_end();
    let body = if line == prefix_without_space && suffix.is_empty() {
        ""
    } else {
        line.strip_prefix(prefix)?
    };
    let body = if suffix.is_empty() {
        body
    } else {
        body.strip_suffix(suffix)?
    };
    Some(if pad_lines {
        body.trim_end().to_owned()
    } else {
        body.to_owned()
    })
}
