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

use std::collections::BTreeMap;
use std::ops::Range;

use serde::Deserialize;

use crate::Error;
use crate::Result;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StyleConfig {
    Line {
        prefix: String,
    },
    Block {
        start: String,
        prefix: String,
        end: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Syntax {
    Line {
        prefix: String,
        marker: String,
    },
    Block {
        start: String,
        start_marker: String,
        prefix: String,
        end: String,
        end_marker: String,
    },
}

/// A validated comment style used for both rendering and structural extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Style {
    name: String,
    syntax: Syntax,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) style: String,
    pub(crate) range: Range<usize>,
    pub(crate) body: String,
    pub(crate) line_count: usize,
}

impl StyleConfig {
    pub(crate) fn resolve(self, name: String) -> Result<Style> {
        let syntax = match self {
            Self::Line { prefix } => {
                reject_line_breaks(&name, "line prefix", &prefix)?;
                let marker = prefix.trim_end().to_owned();
                if marker.is_empty() {
                    return Err(Error::InvalidConfig(format!(
                        "style {name:?} has an empty line prefix"
                    )));
                }
                Syntax::Line { prefix, marker }
            }
            Self::Block { start, prefix, end } => {
                reject_line_breaks(&name, "block start", &start)?;
                reject_line_breaks(&name, "block prefix", &prefix)?;
                reject_line_breaks(&name, "block end", &end)?;
                let start_marker = start.trim_end().to_owned();
                let end_marker = end.trim().to_owned();
                if start_marker.is_empty() || end_marker.is_empty() {
                    return Err(Error::InvalidConfig(format!(
                        "style {name:?} has an empty block delimiter"
                    )));
                }
                Syntax::Block {
                    start,
                    start_marker,
                    prefix,
                    end,
                    end_marker,
                }
            }
        };
        Ok(Style { name, syntax })
    }
}

fn reject_line_breaks(name: &str, field: &str, value: &str) -> Result<()> {
    if value.contains('\r') || value.contains('\n') {
        return Err(Error::InvalidConfig(format!(
            "style {name:?} {field} must fit on one line"
        )));
    }
    Ok(())
}

impl Style {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn render(&self, body: &str, eol: &str) -> String {
        let lines = body_lines(body);
        let mut rendered = String::new();

        match &self.syntax {
            Syntax::Line { prefix, marker } => {
                for line in lines {
                    if line.is_empty() {
                        rendered.push_str(marker);
                    } else {
                        rendered.push_str(prefix);
                        rendered.push_str(line);
                    }
                    rendered.push_str(eol);
                }
            }
            Syntax::Block {
                start, prefix, end, ..
            } => {
                rendered.push_str(start.trim_end());
                rendered.push_str(eol);
                for line in lines {
                    if line.is_empty() {
                        rendered.push_str(prefix.trim_end());
                    } else {
                        rendered.push_str(prefix);
                        rendered.push_str(line);
                    }
                    rendered.push_str(eol);
                }
                rendered.push_str(end.trim_end());
                rendered.push_str(eol);
            }
        }

        rendered.push_str(eol);
        rendered
    }

    pub(crate) fn extract(&self, input: &str, offset: usize) -> Option<Candidate> {
        let start = skip_blank_lines(input, offset);
        match &self.syntax {
            Syntax::Line { marker, .. } => self.extract_line(input, offset, start, marker),
            Syntax::Block {
                start_marker,
                prefix,
                end_marker,
                ..
            } => self.extract_block(input, offset, start, start_marker, prefix, end_marker),
        }
    }

    fn extract_line(
        &self,
        input: &str,
        offset: usize,
        start: usize,
        marker: &str,
    ) -> Option<Candidate> {
        let mut cursor = start;
        let mut lines = Vec::new();

        while let Some(line) = next_line(input, cursor) {
            let Some(body) = line.text.strip_prefix(marker) else {
                break;
            };
            let body = body.strip_prefix(' ').unwrap_or(body).trim_end();
            lines.push(body.to_owned());
            cursor = line.next;
        }

        if lines.is_empty() {
            return None;
        }

        let end = consume_one_blank_line(input, cursor);
        Some(Candidate {
            style: self.name.clone(),
            range: offset..end,
            line_count: lines.len(),
            body: lines.join("\n"),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn extract_block(
        &self,
        input: &str,
        offset: usize,
        start: usize,
        start_marker: &str,
        prefix: &str,
        end_marker: &str,
    ) -> Option<Candidate> {
        let tail = input.get(start..)?;
        let inner_start = tail
            .strip_prefix(start_marker)
            .map(|_| start + start_marker.len())?;
        let relative_end = input.get(inner_start..)?.find(end_marker)?;
        let marker_end = inner_start + relative_end + end_marker.len();
        let closing_line = next_line(input, line_start(input, marker_end));
        let block_line_end = closing_line.as_ref().map_or(marker_end, |line| line.next);

        if !input[marker_end..block_line_end].trim().is_empty() {
            return None;
        }

        let raw_body = &input[inner_start..inner_start + relative_end];
        let lines = strip_block_body(raw_body, prefix);
        let end = consume_one_blank_line(input, block_line_end);
        Some(Candidate {
            style: self.name.clone(),
            range: offset..end,
            line_count: lines.len(),
            body: lines.join("\n"),
        })
    }
}

pub(crate) fn builtin_styles() -> BTreeMap<String, Style> {
    [
        (
            "slash",
            StyleConfig::Line {
                prefix: "// ".to_owned(),
            },
        ),
        (
            "hash",
            StyleConfig::Line {
                prefix: "# ".to_owned(),
            },
        ),
        (
            "dash",
            StyleConfig::Line {
                prefix: "-- ".to_owned(),
            },
        ),
        (
            "slash_star",
            StyleConfig::Block {
                start: "/*".to_owned(),
                prefix: " * ".to_owned(),
                end: " */".to_owned(),
            },
        ),
        (
            "xml",
            StyleConfig::Block {
                start: "<!--".to_owned(),
                prefix: String::new(),
                end: "-->".to_owned(),
            },
        ),
    ]
    .into_iter()
    .map(|(name, config)| {
        let name = name.to_owned();
        let style = config
            .resolve(name.clone())
            .expect("built-in styles must be valid");
        (name, style)
    })
    .collect()
}

pub(crate) fn normalized_body(body: &str) -> String {
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn body_line_count(body: &str) -> usize {
    body_lines(body).len()
}

fn body_lines(body: &str) -> Vec<&str> {
    let mut lines = body.lines().map(str::trim_end).collect::<Vec<_>>();
    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() { vec![""] } else { lines }
}

fn strip_block_body(raw: &str, prefix: &str) -> Vec<String> {
    let marker = prefix.trim();
    let mut lines = raw
        .lines()
        .map(|line| {
            let line = line.trim_end_matches('\r').trim_end();
            if prefix.is_empty() {
                return line.trim().to_owned();
            }
            if let Some(body) = line.strip_prefix(prefix) {
                return body.trim_end().to_owned();
            }
            let trimmed = line.trim_start();
            let body = trimmed.strip_prefix(marker).unwrap_or(trimmed);
            body.strip_prefix(' ').unwrap_or(body).trim_end().to_owned()
        })
        .collect::<Vec<_>>();

    while lines.first().is_some_and(String::is_empty) {
        lines.remove(0);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(crate) struct Line<'a> {
    pub(crate) text: &'a str,
    pub(crate) next: usize,
}

pub(crate) fn next_line(input: &str, start: usize) -> Option<Line<'_>> {
    if start >= input.len() {
        return None;
    }
    let tail = input.get(start..)?;
    let relative_end = tail.find('\n');
    let end = relative_end.map_or(input.len(), |end| start + end);
    let next = relative_end.map_or(input.len(), |_| end + 1);
    let text = input[start..end]
        .strip_suffix('\r')
        .unwrap_or(&input[start..end]);
    Some(Line { text, next })
}

fn skip_blank_lines(input: &str, mut cursor: usize) -> usize {
    while let Some(line) = next_line(input, cursor) {
        if !line.text.trim().is_empty() {
            break;
        }
        cursor = line.next;
    }
    cursor
}

fn consume_one_blank_line(input: &str, cursor: usize) -> usize {
    next_line(input, cursor)
        .filter(|line| line.text.trim().is_empty())
        .map_or(cursor, |line| line.next)
}

fn line_start(input: &str, offset: usize) -> usize {
    input[..offset].rfind('\n').map_or(0, |index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_custom_block_with_an_indented_start_is_readable() {
        let style = StyleConfig::Block {
            start: "  /*".to_owned(),
            prefix: "   * ".to_owned(),
            end: "   */".to_owned(),
        }
        .resolve("indented_block".to_owned())
        .unwrap();
        let rendered = style.render("Copyright 2026 Example", "\n");
        let candidate = style.extract(&rendered, 0).unwrap();

        assert_eq!(candidate.range, 0..rendered.len());
        assert_eq!(candidate.body, "Copyright 2026 Example");
    }

    #[test]
    fn multiline_style_tokens_are_rejected() {
        let result = StyleConfig::Line {
            prefix: "//\n".to_owned(),
        }
        .resolve("broken".to_owned());

        assert!(matches!(result, Err(Error::InvalidConfig(_))));
    }
}
