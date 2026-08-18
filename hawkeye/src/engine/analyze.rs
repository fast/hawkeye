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

use super::Engine;
use super::FileAnalysis;
use super::HeaderTarget;
use super::Replacement;
use super::Rule;
use crate::config::StyleConfig;

impl Engine {
    pub(super) fn analyze(
        &self,
        rule: &Rule,
        input: &str,
        header: &str,
        target: HeaderTarget,
    ) -> FileAnalysis {
        let offset = preamble_offset(input);
        let header_start = skip_blank_lines(input, offset);
        let render = || {
            let eol = line_ending(input);
            let mut rendered = self.style(&rule.style_out).render(header, eol);
            rendered.push_str(eol);
            rendered.push_str(eol);
            rendered
        };

        let matches = rule
            .styles_in
            .iter()
            .filter_map(|name| {
                self.style(name)
                    .parse(input, header_start)
                    .map(|candidate| (name.as_str(), candidate))
            })
            .filter(|(_, candidate)| has_keywords(&candidate.body, &self.keywords));

        let candidate = match unique_style_match(matches) {
            Ok(candidate) => candidate,
            Err(()) => return FileAnalysis::Conflict,
        };

        if let Some((style_name, candidate)) = candidate {
            let end = skip_blank_lines(input, candidate.range.end);
            let candidate_lines = candidate.body.lines().count();
            let header_lines = header.lines().count();
            if !safe_to_replace(&candidate.body, header, &self.keywords)
                || (candidate_lines < header_lines
                    && self
                        .styles
                        .values()
                        .any(|style| style.parse(input, end).is_some()))
            {
                return FileAnalysis::Conflict;
            }
            let range = offset..end;
            if target == HeaderTarget::Absent {
                return FileAnalysis::Remove(Replacement {
                    range,
                    text: String::new(),
                });
            }
            let rendered = render();
            let clean = style_name == rule.style_out
                && candidate.body == header
                && input.get(range.clone()) == Some(rendered.as_str());
            if clean {
                FileAnalysis::Clean
            } else {
                FileAnalysis::Replace(Replacement {
                    range,
                    text: rendered,
                })
            }
        } else if self.styles.values().any(|style| {
            style
                .parse(input, header_start)
                .is_some_and(|candidate| has_keywords(&candidate.body, &self.keywords))
        }) {
            FileAnalysis::Conflict
        } else if target == HeaderTarget::Absent {
            FileAnalysis::Clean
        } else {
            FileAnalysis::Add(Replacement {
                range: offset..header_start,
                text: render(),
            })
        }
    }
}

fn unique_style_match<'a>(
    mut matches: impl Iterator<Item = (&'a str, StyleMatch)>,
) -> Result<Option<(&'a str, StyleMatch)>, ()> {
    let Some((style_name, first)) = matches.next() else {
        return Ok(None);
    };
    for (_, candidate) in matches {
        if candidate.range != first.range || candidate.body != first.body {
            return Err(());
        }
    }
    Ok(Some((style_name, first)))
}

fn has_keywords(body: &str, keywords: &[String]) -> bool {
    let folded = body.to_lowercase();
    keywords.iter().all(|keyword| folded.contains(keyword))
}

fn safe_to_replace(candidate: &str, header: &str, keywords: &[String]) -> bool {
    candidate.lines().count() <= header.lines().count()
        && candidate
            .lines()
            .zip(header.lines())
            .all(|(candidate, header)| {
                if candidate == header {
                    return true;
                }
                let folded = candidate.to_lowercase();
                keywords
                    .iter()
                    .any(|keyword| folded.contains(keyword.as_str()))
            })
}

fn line_ending(input: &str) -> &'static str {
    // Follow the first line ending, as formatter "auto" modes commonly do. This controls only
    // generated header text; the untouched source body is never normalized. A one-line file uses
    // LF as the portable default.
    if input
        .find('\n')
        .is_some_and(|index| index > 0 && input.as_bytes()[index - 1] == b'\r')
    {
        "\r\n"
    } else {
        "\n"
    }
}

fn preamble_offset(input: &str) -> usize {
    // A UTF-8 BOM describes the file itself and must remain before any inserted header.
    let mut position = if input.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let Some((first, line_range)) = lines(input, position).next() else {
        return position;
    };
    // Interpreter and document declarations are meaningful only at the start of a file. `#![` is
    // a Rust inner attribute rather than a shebang, so it remains ordinary source text.
    let lower = first.to_ascii_lowercase();
    if (first.starts_with("#!") && !first.starts_with("#!["))
        || (lower.starts_with("<?xml") && lower.ends_with("?>"))
        || lower
            .strip_prefix("<?php")
            .is_some_and(|tail| tail.chars().next().is_none_or(char::is_whitespace))
        || lower.starts_with("<!doctype ")
        || first.starts_with("%YAML")
        || first.starts_with("%TAG")
    {
        position = line_range.end;
    }

    // YAML permits a consecutive directive block before the document content.
    while let Some((line, line_range)) = lines(input, position).next() {
        if !line.starts_with("%YAML") && !line.starts_with("%TAG") {
            break;
        }
        position = line_range.end;
    }

    // Python and Ruby inspect a small leading window for encoding and other magic comments. The
    // bound prevents an ordinary leading comment later in the file from becoming a preamble.
    for _ in 0..2 {
        let Some((line, line_range)) = lines(input, position).next() else {
            break;
        };
        let lower = line.to_ascii_lowercase();
        let magic = line.starts_with('#')
            && (lower.contains("coding:")
                || lower.contains("coding=")
                || lower.contains("frozen_string_literal:")
                || lower.contains("-*-"));
        if !magic {
            break;
        }
        position = line_range.end;
    }
    position
}

fn skip_blank_lines(input: &str, mut position: usize) -> usize {
    for (line, line_range) in lines(input, position) {
        if !line.trim().is_empty() {
            break;
        }
        position = line_range.end;
    }
    position
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StyleMatch {
    range: Range<usize>,
    body: String,
}

impl StyleConfig {
    fn render(&self, body: &str, eol: &str) -> String {
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

    fn parse(&self, input: &str, start: usize) -> Option<StyleMatch> {
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
    for (line, line_range) in lines(input, start) {
        let Some(content) = strip_affixes(line, prefix, suffix, pad_lines) else {
            break;
        };
        body.push(content);
        end = line_range.start + line.len();
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
    let mut lines = lines(input, start);
    let (first, _) = lines.next()?;
    if first != opening {
        return None;
    }

    let mut body = Vec::new();
    for (content, line_range) in lines {
        if content == closing {
            let end = line_range.start + content.len();
            return Some(StyleMatch {
                range: start..end,
                body: body.join("\n"),
            });
        }
        body.push(strip_affixes(content, prefix, suffix, false)?);
    }
    None
}

fn strip_affixes<'a>(
    line: &'a str,
    prefix: &str,
    suffix: &str,
    pad_lines: bool,
) -> Option<&'a str> {
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
    Some(if pad_lines { body.trim_end() } else { body })
}

/// Iterates line contents without terminators and their full byte ranges in the input.
fn lines(input: &str, start: usize) -> impl Iterator<Item = (&str, Range<usize>)> {
    let mut position = start;
    input[start..].split_inclusive('\n').map(move |line| {
        let line_range = position..position + line.len();
        position = line_range.end;
        let content = if let Some(content) = line.strip_suffix('\n') {
            content.strip_suffix('\r').unwrap_or(content)
        } else {
            line
        };
        (content, line_range)
    })
}
