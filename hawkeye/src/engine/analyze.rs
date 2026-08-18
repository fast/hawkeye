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
use super::Rule;
use super::Target;
use super::lines;
use super::style::StyleMatch;
use crate::report::FileOutcome;

pub struct FileAnalysis {
    pub outcome: FileOutcome,
    pub replacement: Option<Replacement>,
}

pub struct Replacement {
    range: Range<usize>,
    value: String,
}

impl Replacement {
    pub fn apply(&self, input: &mut String) {
        input.replace_range(self.range.clone(), &self.value);
    }
}

impl Engine {
    pub(super) fn analyze(
        &self,
        rule: &Rule,
        input: &str,
        header: &str,
        target: Target,
    ) -> FileAnalysis {
        let offset = preamble_offset(input);
        let header_start = skip_blank_lines(input, offset);
        let eol = detect_eol(input);
        let rendered = {
            let mut value = self.style(&rule.style_out).render(header, eol);
            value.push_str(eol);
            value.push_str(eol);
            value
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
            Err(()) => {
                return FileAnalysis {
                    outcome: FileOutcome::Conflict,
                    replacement: None,
                };
            }
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
                return FileAnalysis {
                    outcome: FileOutcome::Conflict,
                    replacement: None,
                };
            }
            let range = offset..end;
            if target == Target::Absent {
                return FileAnalysis {
                    outcome: FileOutcome::Remove,
                    replacement: Some(Replacement {
                        range,
                        value: String::new(),
                    }),
                };
            }
            let clean = style_name == rule.style_out
                && candidate.body == header
                && input.get(range.clone()) == Some(rendered.as_str());
            if clean {
                FileAnalysis {
                    outcome: FileOutcome::Clean,
                    replacement: None,
                }
            } else {
                FileAnalysis {
                    outcome: FileOutcome::Replace,
                    replacement: Some(Replacement {
                        range,
                        value: rendered,
                    }),
                }
            }
        } else if self.styles.values().any(|style| {
            style
                .parse(input, header_start)
                .is_some_and(|candidate| has_keywords(&candidate.body, &self.keywords))
        }) {
            FileAnalysis {
                outcome: FileOutcome::Conflict,
                replacement: None,
            }
        } else if target == Target::Absent {
            FileAnalysis {
                outcome: FileOutcome::Clean,
                replacement: None,
            }
        } else {
            FileAnalysis {
                outcome: FileOutcome::Add,
                replacement: Some(Replacement {
                    range: offset..header_start,
                    value: rendered,
                }),
            }
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
    let Some((first, range)) = lines::iter(input, position).next() else {
        return position;
    };
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
        position = range.end;
    }

    while let Some((line, range)) = lines::iter(input, position).next() {
        if !line.starts_with("%YAML") && !line.starts_with("%TAG") {
            break;
        }
        position = range.end;
    }

    for _ in 0..2 {
        let Some((line, range)) = lines::iter(input, position).next() else {
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
        position = range.end;
    }
    position
}

fn skip_blank_lines(input: &str, mut position: usize) -> usize {
    for (line, range) in lines::iter(input, position) {
        if !line.trim().is_empty() {
            break;
        }
        position = range.end;
    }
    position
}
