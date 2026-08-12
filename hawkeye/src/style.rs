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

use crate::config::StyleConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Style {
    name: String,
    syntax: Syntax,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Syntax {
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
pub(crate) struct Candidate {
    pub(crate) style: String,
    pub(crate) range: Range<usize>,
    pub(crate) body: String,
}

impl Style {
    pub(crate) fn from_config(name: String, config: StyleConfig) -> Self {
        let syntax = match config {
            StyleConfig::Line {
                prefix,
                suffix,
                pad_lines,
            } => Syntax::Line {
                prefix,
                suffix,
                pad_lines,
            },
            StyleConfig::Block {
                start,
                prefix,
                suffix,
                end,
            } => Syntax::Block {
                start,
                prefix,
                suffix,
                end,
            },
        };
        Self { name, syntax }
    }

    pub(crate) fn render(&self, body: &str, eol: &str) -> String {
        let lines = body.split('\n').collect::<Vec<_>>();
        let mut output = String::new();
        match &self.syntax {
            Syntax::Line {
                prefix,
                suffix,
                pad_lines,
            } => {
                let width = lines
                    .iter()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0);
                for line in lines {
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
                        truncate_trailing_spaces(&mut output);
                    }
                    output.push_str(eol);
                }
            }
            Syntax::Block {
                start,
                prefix,
                suffix,
                end,
            } => {
                output.push_str(start);
                output.push_str(eol);
                for line in lines {
                    output.push_str(prefix);
                    output.push_str(line);
                    output.push_str(suffix);
                    if suffix.is_empty() {
                        truncate_trailing_spaces(&mut output);
                    }
                    output.push_str(eol);
                }
                output.push_str(end);
                output.push_str(eol);
            }
        }
        output
    }

    pub(crate) fn extract(&self, input: &str, offset: usize) -> Option<Candidate> {
        let start = skip_blank_lines(input, offset);
        let (range, body) = match &self.syntax {
            Syntax::Line {
                prefix,
                suffix,
                pad_lines,
            } => extract_line(input, start, prefix, suffix, *pad_lines)?,
            Syntax::Block {
                start: opening,
                prefix,
                suffix,
                end: closing,
            } => extract_block(input, start, opening, prefix, suffix, closing)?,
        };
        Some(Candidate {
            style: self.name.clone(),
            range: offset..range.end,
            body,
        })
    }
}

pub(crate) fn builtin_styles() -> BTreeMap<String, Style> {
    let configs = [
        ("slash_line", line("// ", "", false)),
        ("triple_slash_line", line("/// ", "", false)),
        ("hash_line", line("# ", "", false)),
        ("dash_line", line("-- ", "", false)),
        ("percent_line", line("% ", "", false)),
        ("percent3_line", line("%%% ", "", false)),
        ("semicolon_line", line("; ", "", false)),
        ("apostrophe_line", line("' ", "", false)),
        ("bang_line", line("! ", "", false)),
        ("bang3_line", line("!!! ", "", false)),
        ("tilde2_line", line("~~ ", "", false)),
        ("rem_line", line("@REM ", "", false)),
        ("haml_line", line("-# ", "", false)),
        ("xml_line", line("<!-- ", " -->", true)),
        ("slash_block", block("/*", " * ", "", " */")),
        ("javadoc_block", block("/**", " * ", "", " */")),
        ("xml_block", block("<!--", "    ", "", "-->")),
        ("lua_block", block("--[[", "    ", "", "]]")),
        ("brace_star_block", block("{*", " * ", "", " *}")),
        ("hash_star_block", block("#*", " * ", "", " *#")),
        ("mustache_block", block("{{!", "    ", "", "}}")),
        ("mvel_block", block("@comment{", "  ", "", "}")),
        ("freemarker_block", block("<#--", "    ", "", "-->")),
        ("freemarker_alt_block", block("[#--", "    ", "", "--]")),
        ("jsp_block", block("<%--", "    ", "", "--%>")),
        ("coldfusion_block", block("<!---", "    ", "", "--->")),
        ("asp_block", block("<%", "' ", "", "%>")),
        (
            "swift_banner",
            block(
                "//===----------------------------------------------------------------------===//",
                "// ",
                "",
                "//===----------------------------------------------------------------------===//",
            ),
        ),
        ("asciidoc_block", block("////", "// ", "", "////")),
    ];
    configs
        .into_iter()
        .map(|(name, config)| (name.to_owned(), Style::from_config(name.to_owned(), config)))
        .collect()
}

fn line(prefix: &str, suffix: &str, pad_lines: bool) -> StyleConfig {
    StyleConfig::Line {
        prefix: prefix.to_owned(),
        suffix: suffix.to_owned(),
        pad_lines,
    }
}

fn block(start: &str, prefix: &str, suffix: &str, end: &str) -> StyleConfig {
    StyleConfig::Block {
        start: start.to_owned(),
        prefix: prefix.to_owned(),
        suffix: suffix.to_owned(),
        end: end.to_owned(),
    }
}

fn extract_line(
    input: &str,
    start: usize,
    prefix: &str,
    suffix: &str,
    pad_lines: bool,
) -> Option<(Range<usize>, String)> {
    let mut position = start;
    let mut lines = Vec::new();
    while let Some(line) = next_line(input, position) {
        let Some(body) = unwrap_line(line.content, prefix, suffix, pad_lines) else {
            break;
        };
        lines.push(body);
        position = line.end;
        if line.end == input.len() {
            break;
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some((start..position, lines.join("\n")))
    }
}

fn extract_block(
    input: &str,
    start: usize,
    opening: &str,
    prefix: &str,
    suffix: &str,
    closing: &str,
) -> Option<(Range<usize>, String)> {
    let first = next_line(input, start)?;
    if first.content != opening {
        return None;
    }

    let mut position = first.end;
    let mut lines = Vec::new();
    while let Some(line) = next_line(input, position) {
        if line.content == closing {
            return Some((start..line.end, lines.join("\n")));
        }
        lines.push(unwrap_line(line.content, prefix, suffix, false)?);
        position = line.end;
        if line.end == input.len() {
            break;
        }
    }
    None
}

fn unwrap_line(line: &str, prefix: &str, suffix: &str, pad_lines: bool) -> Option<String> {
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

pub(crate) fn skip_blank_lines(input: &str, mut position: usize) -> usize {
    while let Some(line) = next_line(input, position) {
        if !line.content.trim().is_empty() {
            break;
        }
        position = line.end;
        if position == input.len() {
            break;
        }
    }
    position
}

fn truncate_trailing_spaces(output: &mut String) {
    let trimmed = output.trim_end_matches([' ', '\t']).len();
    output.truncate(trimmed);
}

#[derive(Debug, Clone, Copy)]
struct Line<'input> {
    content: &'input str,
    end: usize,
}

fn next_line(input: &str, position: usize) -> Option<Line<'_>> {
    if position >= input.len() {
        return None;
    }
    let tail = &input[position..];
    if let Some(relative_end) = tail.find('\n') {
        let mut content_end = position + relative_end;
        if input.as_bytes().get(content_end.wrapping_sub(1)) == Some(&b'\r') {
            content_end -= 1;
        }
        Some(Line {
            content: &input[position..content_end],
            end: position + relative_end + 1,
        })
    } else {
        Some(Line {
            content: tail,
            end: input.len(),
        })
    }
}
