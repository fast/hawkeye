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

use super::Analysis;
use super::Engine;
use crate::edit::Edit;
use crate::report::Mode;
use crate::report::Status;
use crate::style::Candidate;
use crate::style::next_line;
use crate::style::skip_blank_lines;

impl Engine {
    pub(super) fn analyze(&self, path: &Path, input: &str, header: &str, mode: Mode) -> Analysis {
        let Some(rule) = self.rule_for(path) else {
            return Analysis {
                status: Status::Unsupported,
                edit: None,
            };
        };

        let offset = preamble_offset(input);
        let eol = detect_eol(input);
        let rendered = {
            let mut value = self.style(&rule.style_out).render(header, eol);
            value.push_str(eol);
            value
        };

        let candidates = rule
            .styles_in
            .iter()
            .filter_map(|name| {
                self.style(name)
                    .extract(input, offset)
                    .map(|candidate| (name.as_str(), candidate))
            })
            .filter(|(_, candidate)| has_keywords(&candidate.body, &self.keywords))
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

        if let Some((style_name, candidate)) = candidate {
            let candidate_lines = candidate.body.lines().count();
            let header_lines = header.lines().count();
            if !safe_to_replace(&candidate.body, header, &self.keywords)
                || (candidate_lines < header_lines
                    && self
                        .styles
                        .values()
                        .any(|style| style.extract(input, candidate.range.end).is_some()))
            {
                return Analysis {
                    status: Status::Conflict,
                    edit: None,
                };
            }
            let end = skip_blank_lines(input, candidate.range.end);
            let range = candidate.range.start..end;
            if mode == Mode::Remove {
                return Analysis {
                    status: Status::Replaceable,
                    edit: Some(Edit::new(range, String::new())),
                };
            }
            let clean = style_name == rule.style_out
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
        } else if self.styles.values().any(|style| {
            style
                .extract(input, offset)
                .is_some_and(|candidate| has_keywords(&candidate.body, &self.keywords))
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
}

fn unique_candidate(candidates: Vec<(&str, Candidate)>) -> Result<Option<(&str, Candidate)>, ()> {
    let mut candidates = candidates.into_iter();
    let Some((style_name, first)) = candidates.next() else {
        return Ok(None);
    };
    for (_, candidate) in candidates {
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
    let candidate_lines = candidate.lines().collect::<Vec<_>>();
    let header_lines = header.lines().collect::<Vec<_>>();
    candidate_lines.len() <= header_lines.len()
        && candidate_lines
            .iter()
            .zip(header_lines)
            .all(|(candidate, header)| {
                if candidate == &header {
                    return true;
                }
                let folded = candidate.to_lowercase();
                keywords
                    .iter()
                    .any(|keyword| folded.contains(keyword.as_str()))
            })
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
