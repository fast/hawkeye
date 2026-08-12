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

use std::path::Path;

use crate::ResolvedConfig;
use crate::edit::Edit;
use crate::report::Mode;
use crate::report::Status;
use crate::style::Candidate;
use crate::style::consume_blank_lines;
use crate::style::skip_blank_lines;

pub(crate) struct Analysis {
    pub(crate) status: Status,
    pub(crate) edit: Option<Edit>,
}

pub(crate) fn analyze(
    config: &ResolvedConfig,
    path: &Path,
    input: &str,
    header: &str,
    mode: Mode,
) -> Analysis {
    let Some(rule) = config.rule_for(path) else {
        return Analysis {
            status: Status::Unsupported,
            edit: None,
        };
    };

    let offset = preamble_offset(input);
    let eol = detect_eol(input);
    let rendered = {
        let mut value = config.style(rule.style_out()).render(header, eol);
        value.push_str(eol);
        value
    };

    let candidates = rule
        .styles_in()
        .iter()
        .filter_map(|name| config.style(name).extract(input, offset))
        .filter(|candidate| has_keywords(&candidate.body, config.keywords()))
        .collect::<Vec<_>>();

    let candidate = match unique_candidate(candidates) {
        Ok(candidate) => candidate,
        Err(()) => {
            return Analysis {
                status: Status::Conflict,
                edit: None,
            };
        }
    };

    if let Some(candidate) = candidate {
        let candidate_lines = candidate.body.lines().count();
        let header_lines = header.lines().count();
        if (config.style(&candidate.style).is_line() && candidate_lines > header_lines)
            || (candidate_lines < header_lines
                && config
                    .styles()
                    .any(|style| style.extract(input, candidate.range.end).is_some()))
        {
            return Analysis {
                status: Status::Conflict,
                edit: None,
            };
        }
        let end = consume_blank_lines(input, candidate.range.end);
        let range = candidate.range.start..end;
        if mode == Mode::Remove {
            return Analysis {
                status: Status::Replaceable,
                edit: Some(Edit::new(range, String::new())),
            };
        }
        let clean = candidate.style == rule.style_out()
            && candidate.body == header
            && input.get(range.clone()) == Some(rendered.as_str());
        if clean {
            Analysis {
                status: Status::Clean,
                edit: None,
            }
        } else {
            Analysis {
                status: Status::Replaceable,
                edit: (mode == Mode::Format).then(|| Edit::new(range, rendered)),
            }
        }
    } else if config.styles().any(|style| {
        style
            .extract(input, offset)
            .is_some_and(|candidate| has_keywords(&candidate.body, config.keywords()))
    }) {
        Analysis {
            status: Status::Conflict,
            edit: None,
        }
    } else {
        let leading_end = skip_blank_lines(input, offset);
        Analysis {
            status: Status::Missing,
            edit: (mode == Mode::Format).then(|| Edit::new(offset..leading_end, rendered)),
        }
    }
}

fn unique_candidate(candidates: Vec<Candidate>) -> Result<Option<Candidate>, ()> {
    let mut candidates = candidates.into_iter();
    let Some(first) = candidates.next() else {
        return Ok(None);
    };
    for candidate in candidates {
        if candidate.range != first.range || candidate.body != first.body {
            return Err(());
        }
    }
    Ok(Some(first))
}

fn has_keywords(body: &str, keywords: &[String]) -> bool {
    let folded = body.to_lowercase();
    keywords.iter().all(|keyword| folded.contains(keyword))
}

fn detect_eol(input: &str) -> &'static str {
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
    let mut position = usize::from(input.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let Some(first) = next_line(input, position) else {
        return position;
    };
    let lower = first.content.to_ascii_lowercase();
    if (first.content.starts_with("#!") && !first.content.starts_with("#!["))
        || (lower.starts_with("<?xml") && lower.ends_with("?>"))
        || lower
            .strip_prefix("<?php")
            .is_some_and(|tail| tail.chars().next().is_none_or(char::is_whitespace))
        || lower.starts_with("<!doctype ")
        || first.content.starts_with("%YAML")
        || first.content.starts_with("%TAG")
    {
        position = first.end;
    }

    for _ in 0..2 {
        let Some(line) = next_line(input, position) else {
            break;
        };
        let lower = line.content.to_ascii_lowercase();
        let magic = line.content.starts_with('#')
            && (lower.contains("coding:")
                || lower.contains("coding=")
                || lower.contains("frozen_string_literal:")
                || lower.contains("-*-"));
        if !magic {
            break;
        }
        position = line.end;
    }
    position
}

#[derive(Clone, Copy)]
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
