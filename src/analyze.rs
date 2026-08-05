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
use std::path::Path;

use crate::Edit;
use crate::EditPlan;
use crate::Error;
use crate::Mode;
use crate::ResolvedConfig;
use crate::Result;
use crate::Status;
use crate::config::Rule;
use crate::style::Candidate;
use crate::style::body_line_count;
use crate::style::normalized_body;

/// Pure, in-memory license-header analysis over a resolved configuration.
pub struct Analyzer<'config> {
    config: &'config ResolvedConfig,
    header_body: String,
    normalized_header: String,
    header_line_count: usize,
}

enum Identity {
    Header,
    Conflict,
    Unrelated,
}

struct ProvenHeader {
    range: Range<usize>,
    style: String,
    count: usize,
}

impl<'config> Analyzer<'config> {
    /// Creates an analyzer from validated configuration and loaded unstyled header text.
    pub fn new(config: &'config ResolvedConfig, header_body: impl Into<String>) -> Result<Self> {
        let header_body = header_body.into();
        if header_body.trim().is_empty() {
            return Err(Error::InvalidConfig(
                "loaded header text cannot be empty".to_owned(),
            ));
        }
        let normalized_header = normalized_body(&header_body);
        let header_line_count = body_line_count(&header_body);
        Ok(Self {
            config,
            header_body,
            normalized_header,
            header_line_count,
        })
    }

    /// Analyzes one relative path and creates the safe edit for `mode`, if one exists.
    pub fn plan(&self, path: impl AsRef<Path>, input: &str, mode: Mode) -> Result<EditPlan> {
        let Some(rule) = self.config.rule_for(path.as_ref()) else {
            return Ok(EditPlan::new(Status::Unsupported, None));
        };

        let preamble = preamble(input);
        let insertion = preamble.end;
        let eol = detect_eol(input);
        let write_style = self.config.style(rule.write_style());
        let mut rendered = write_style.render(&self.header_body, eol);
        if preamble.needs_separator {
            rendered.insert_str(0, eol);
        }
        let analysis = self.find_headers(input, insertion, rule);

        let (status, proven_range) = match analysis {
            HeaderScan::Missing => (Status::Missing, None),
            HeaderScan::Conflict => (Status::Conflict, None),
            HeaderScan::Proven(header) => {
                let exact = header.count == 1
                    && header.style == write_style.name()
                    && input[header.range.clone()] == rendered;
                (
                    if exact {
                        Status::Clean
                    } else {
                        Status::Replaceable
                    },
                    Some(header.range),
                )
            }
        };

        let edit = match (mode, status) {
            (Mode::Format, Status::Missing) => {
                Some(Edit::new(input, insertion..insertion, rendered)?)
            }
            (Mode::Format, Status::Replaceable) => Some(Edit::new(
                input,
                proven_range.expect("replaceable headers have a proven range"),
                rendered,
            )?),
            (Mode::Remove, Status::Clean | Status::Replaceable) => Some(Edit::new(
                input,
                proven_range.expect("removable headers have a proven range"),
                String::new(),
            )?),
            _ => None,
        };

        Ok(EditPlan::new(status, edit))
    }

    fn find_headers(&self, input: &str, insertion: usize, rule: &Rule) -> HeaderScan {
        let Some(first) = self.find_allowed(input, insertion, rule) else {
            return if self.has_conflict(input, insertion) {
                HeaderScan::Conflict
            } else {
                HeaderScan::Missing
            };
        };

        let (header, identity) = first;
        match identity {
            Identity::Header => {}
            Identity::Conflict => return HeaderScan::Conflict,
            Identity::Unrelated => {
                return if self.has_conflict(input, insertion) {
                    HeaderScan::Conflict
                } else {
                    HeaderScan::Missing
                };
            }
        }

        let mut proven = ProvenHeader {
            range: header.range.clone(),
            style: header.style.clone(),
            count: 1,
        };

        loop {
            let cursor = proven.range.end;
            if let Some((next, identity)) = self.find_allowed(input, cursor, rule) {
                match identity {
                    Identity::Header => {
                        proven.range.end = next.range.end;
                        proven.count += 1;
                        continue;
                    }
                    Identity::Conflict => return HeaderScan::Conflict,
                    Identity::Unrelated => {}
                }
            }

            if self.has_conflict(input, cursor) {
                return HeaderScan::Conflict;
            }
            break;
        }

        HeaderScan::Proven(proven)
    }

    fn find_allowed(
        &self,
        input: &str,
        offset: usize,
        rule: &Rule,
    ) -> Option<(Candidate, Identity)> {
        let mut unrelated = None;
        for name in rule.read_styles() {
            let Some(candidate) = self.config.style(name).extract(input, offset) else {
                continue;
            };
            let identity = self.identity(&candidate);
            if matches!(identity, Identity::Header | Identity::Conflict) {
                return Some((candidate, identity));
            }
            unrelated.get_or_insert((candidate, identity));
        }
        unrelated
    }

    fn has_conflict(&self, input: &str, offset: usize) -> bool {
        let mut cursor = offset;
        loop {
            let mut unrelated_end = None;
            for style in self.config.styles() {
                let Some(candidate) = style.extract(input, cursor) else {
                    continue;
                };
                if !matches!(self.identity(&candidate), Identity::Unrelated) {
                    return true;
                }
                unrelated_end = Some(unrelated_end.map_or(candidate.range.end, |end: usize| {
                    end.max(candidate.range.end)
                }));
            }
            let Some(next) = unrelated_end.filter(|next| *next > cursor) else {
                return false;
            };
            cursor = next;
        }
    }

    fn identity(&self, candidate: &Candidate) -> Identity {
        if normalized_body(&candidate.body) == self.normalized_header {
            return Identity::Header;
        }

        let body = candidate.body.to_lowercase();
        let identified = !self.config.header().identifiers().is_empty()
            && self
                .config
                .header()
                .identifiers()
                .iter()
                .all(|identifier| body.contains(identifier));
        if !identified {
            return Identity::Unrelated;
        }

        if candidate.line_count == self.header_line_count {
            Identity::Header
        } else {
            Identity::Conflict
        }
    }
}

enum HeaderScan {
    Missing,
    Conflict,
    Proven(ProvenHeader),
}

fn detect_eol(input: &str) -> &'static str {
    input.find('\n').map_or("\n", |index| {
        if input[..index].ends_with('\r') {
            "\r\n"
        } else {
            "\n"
        }
    })
}

struct Preamble {
    end: usize,
    needs_separator: bool,
}

fn preamble(input: &str) -> Preamble {
    let mut offset = if input.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let tail = &input[offset..];
    let lower = tail.get(..5).unwrap_or(tail).to_ascii_lowercase();
    let is_shebang = tail.starts_with("#!") && !tail.starts_with("#![");
    let is_xml = lower.starts_with("<?xml");
    let is_php = lower.starts_with("<?php");

    if is_shebang || is_xml || is_php {
        if let Some(index) = tail.find('\n') {
            offset += index + 1;
            return Preamble {
                end: offset,
                needs_separator: false,
            };
        }
        offset += tail.len();
        return Preamble {
            end: offset,
            needs_separator: true,
        };
    }

    Preamble {
        end: offset,
        needs_separator: false,
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    use super::*;

    const HEADER: &str = "Copyright 2026 FastLabs Developers";

    fn config(write_style: &str, read_styles: &[&str], pattern: &str) -> ResolvedConfig {
        let read_styles = read_styles
            .iter()
            .map(|style| format!("\"{style}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let input = format!(
            r#"
[header]
text = "{HEADER}"
identifiers = ["Copyright"]

[[rules]]
patterns = ["{pattern}"]
write_style = "{write_style}"
read_styles = [{read_styles}]
"#
        );
        Config::from_toml(&input).unwrap().resolve(".").unwrap()
    }

    fn format(input: &str, config: &ResolvedConfig, path: &str) -> (Status, String) {
        let plan = Analyzer::new(config, HEADER)
            .unwrap()
            .plan(path, input, Mode::Format)
            .unwrap();
        (plan.status(), plan.apply(input).unwrap())
    }

    #[test]
    fn preferred_header_is_clean() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = "// Copyright 2026 FastLabs Developers\n\nfn main() {}\n";
        let plan = Analyzer::new(&config, HEADER)
            .unwrap()
            .plan("src/main.rs", input, Mode::Format)
            .unwrap();
        assert_eq!(plan.status(), Status::Clean);
        assert!(plan.edit().is_none());
    }

    #[test]
    fn stale_header_is_replaced_and_formatting_is_idempotent() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = "// Copyright 2025 FastLabs Developers\n\nfn main() {}\n";
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Replaceable);
        assert_eq!(
            output,
            "// Copyright 2026 FastLabs Developers\n\nfn main() {}\n"
        );
        assert_eq!(format(&output, &config, "src/main.rs").0, Status::Clean);
    }

    #[test]
    fn accepted_block_style_migrates_to_preferred_line_style() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = "/*\n * Copyright 2025 FastLabs Developers\n */\n\nfn main() {}\n";
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Replaceable);
        assert_eq!(
            output,
            "// Copyright 2026 FastLabs Developers\n\nfn main() {}\n"
        );
    }

    #[test]
    fn multi_line_block_header_migrates_without_duplication() {
        let raw = r#"
[header]
text = """Licensed to the Apache Software Foundation (ASF) under one
or more contributor license agreements."""
identifiers = ["Apache Software Foundation", "contributor license agreements"]

[[rules]]
patterns = ["**/*.rs"]
write_style = "slash"
read_styles = ["slash_star"]
"#;
        let config = Config::from_toml(raw).unwrap().resolve(".").unwrap();
        let header = concat!(
            "Licensed to the Apache Software Foundation (ASF) under one\n",
            "or more contributor license agreements."
        );
        let input = concat!(
            "/*\n",
            " * Licensed to the Apache Software Foundation (ASF) under one\n",
            " * or more contributor license agreements.\n",
            " */\n\n",
            "fn main() {}\n"
        );

        let first = Analyzer::new(&config, header)
            .unwrap()
            .plan("src/main.rs", input, Mode::Format)
            .unwrap();
        let output = first.apply(input).unwrap();
        let second = Analyzer::new(&config, header)
            .unwrap()
            .plan("src/main.rs", &output, Mode::Format)
            .unwrap();

        assert_eq!(first.status(), Status::Replaceable);
        assert_eq!(output.matches("Apache Software Foundation").count(), 1);
        assert!(!output.contains("/*"));
        assert_eq!(second.status(), Status::Clean);
        assert_eq!(second.apply(&output).unwrap(), output);
    }

    #[test]
    fn unlisted_license_style_is_a_non_mutating_conflict() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = "# Copyright 2025 FastLabs Developers\n\nfn main() {}\n";
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Conflict);
        assert_eq!(output, input);
    }

    #[test]
    fn identifier_in_code_does_not_look_like_a_header() {
        let config = config("slash", &[], "**/*.rs");
        let input = "const NOTICE: &str = \"Copyright 2026\";\n";
        let (status, output) = format(input, &config, "src/lib.rs");
        assert_eq!(status, Status::Missing);
        assert!(output.starts_with("// Copyright 2026 FastLabs Developers\n\n"));
    }

    #[test]
    fn stacked_accepted_headers_collapse_to_one() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = concat!(
            "/*\n * Copyright 2025 FastLabs Developers\n */\n\n",
            "// Copyright 2024 FastLabs Developers\n\n",
            "fn main() {}\n"
        );
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Replaceable);
        assert_eq!(output.matches("Copyright").count(), 1);
        assert!(!output.contains("/*"));
    }

    #[test]
    fn stacked_accepted_headers_collapse_in_either_order() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = concat!(
            "// Copyright 2024 FastLabs Developers\n\n",
            "/*\n * Copyright 2025 FastLabs Developers\n */\n\n",
            "fn main() {}\n"
        );
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Replaceable);
        assert_eq!(output.matches("Copyright").count(), 1);
        assert!(!output.contains("/*"));
    }

    #[test]
    fn listed_header_stacked_with_unlisted_header_is_a_conflict() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = concat!(
            "/*\n * Copyright 2025 FastLabs Developers\n */\n\n",
            "# Copyright 2024 FastLabs Developers\n\n",
            "fn main() {}\n"
        );
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Conflict);
        assert_eq!(output, input);
    }

    #[test]
    fn license_after_an_unrelated_leading_comment_is_a_conflict() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = concat!(
            "// Generated source metadata.\n\n",
            "/*\n * Copyright 2025 FastLabs Developers\n */\n\n",
            "fn main() {}\n"
        );
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Conflict);
        assert_eq!(output, input);
    }

    #[test]
    fn shebang_stays_before_inserted_header() {
        let config = config("hash", &[], "**/*.sh");
        let input = "#!/usr/bin/env bash\necho hello\n";
        let (status, output) = format(input, &config, "bin/run.sh");
        assert_eq!(status, Status::Missing);
        assert_eq!(
            output,
            "#!/usr/bin/env bash\n# Copyright 2026 FastLabs Developers\n\necho hello\n"
        );
    }

    #[test]
    fn unterminated_preamble_is_separated_from_the_inserted_header() {
        let config = config("hash", &[], "**/*.sh");
        let input = "#!/usr/bin/env bash";
        let (status, output) = format(input, &config, "bin/run.sh");
        assert_eq!(status, Status::Missing);
        assert_eq!(
            output,
            "#!/usr/bin/env bash\n# Copyright 2026 FastLabs Developers\n\n"
        );
    }

    #[test]
    fn xml_declaration_stays_before_inserted_header() {
        let config = config("xml", &[], "**/*.xml");
        let input = "<?xml version=\"1.0\"?>\n<root/>\n";
        let (status, output) = format(input, &config, "config/main.xml");
        assert_eq!(status, Status::Missing);
        assert_eq!(
            output,
            concat!(
                "<?xml version=\"1.0\"?>\n",
                "<!--\nCopyright 2026 FastLabs Developers\n-->\n\n",
                "<root/>\n"
            )
        );
    }

    #[test]
    fn remove_uses_only_a_proven_range() {
        let config = config("slash", &["slash_star"], "**/*.rs");
        let input = "// Copyright 2025 FastLabs Developers\n\nfn main() {}\n";
        let plan = Analyzer::new(&config, HEADER)
            .unwrap()
            .plan("src/main.rs", input, Mode::Remove)
            .unwrap();
        assert_eq!(plan.status(), Status::Replaceable);
        assert_eq!(plan.apply(input).unwrap(), "fn main() {}\n");
    }

    #[test]
    fn identified_comment_with_a_different_shape_is_a_conflict() {
        let config = config("slash", &[], "**/*.rs");
        let input = concat!(
            "// Copyright 2025 FastLabs Developers\n",
            "// Documentation that must not be deleted.\n\n",
            "fn main() {}\n"
        );
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Conflict);
        assert_eq!(output, input);
    }

    #[test]
    fn inserted_header_preserves_crlf() {
        let config = config("slash", &[], "**/*.rs");
        let input = "fn main() {}\r\n";
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Missing);
        assert_eq!(
            output,
            "// Copyright 2026 FastLabs Developers\r\n\r\nfn main() {}\r\n"
        );
    }

    #[test]
    fn byte_order_mark_stays_before_inserted_header() {
        let config = config("slash", &[], "**/*.rs");
        let input = "\u{feff}fn main() {}\n";
        let (status, output) = format(input, &config, "src/main.rs");
        assert_eq!(status, Status::Missing);
        assert_eq!(
            output,
            "\u{feff}// Copyright 2026 FastLabs Developers\n\nfn main() {}\n"
        );
    }

    #[test]
    fn unmatched_path_is_unsupported() {
        let config = config("slash", &[], "**/*.rs");
        let plan = Analyzer::new(&config, HEADER)
            .unwrap()
            .plan("README.md", "text\n", Mode::Format)
            .unwrap();
        assert_eq!(plan.status(), Status::Unsupported);
        assert!(plan.edit().is_none());
    }
}
