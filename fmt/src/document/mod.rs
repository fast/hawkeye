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

use crate::config::ExistingStrategy;
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
    removal_candidates: Vec<HeaderDef>,
    keywords: Vec<String>,
    existing_strategy: ExistingStrategy,
    props: HashMap<String, String>,
    attrs: Attributes,
    parser: HeaderParser,
}

impl Document {
    pub fn new(
        filepath: PathBuf,
        header_def: HeaderDef,
        removal_candidates: Vec<HeaderDef>,
        keywords: Vec<String>,
        existing_strategy: ExistingStrategy,
        props: HashMap<String, String>,
        attrs: Attributes,
    ) -> Result<Option<Self>, Error> {
        match FileContent::new(&filepath) {
            Ok(content) => Ok(Some(Self {
                parser: parse_header(content, &header_def, &keywords),
                filepath,
                header_def,
                removal_candidates,
                keywords,
                existing_strategy,
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

    pub fn existing_strategy(&self) -> ExistingStrategy {
        self.existing_strategy
    }

    /// Style-agnostic probe: does the file's prefix carry a license keyword as a whole word,
    /// in any comment style? Looser than [`Self::header_detected`] so `format` reports an
    /// unlisted-style header instead of stacking a duplicate. A hit in a string literal or
    /// prose still classifies as foreign on purpose: controlled failure over a silent miss.
    pub fn looks_licensed(&self) -> bool {
        // Char cap, not line cap: still catches a notice pushed down by a long preamble yet
        // stays cheap on a minified single-line file. `chars().take` is panic-safe (no UTF-8
        // boundary slicing). `keywords` are lowercased at construction; lowercase the prefix too.
        const SCAN_CHARS: usize = 16 * 1024;
        let prefix: String = self
            .parser
            .file_content
            .content()
            .chars()
            .take(SCAN_CHARS)
            .flat_map(char::to_lowercase)
            .collect();
        self.keywords
            .iter()
            .any(|kw| contains_whole_word(&prefix, kw))
    }

    /// True if the file already carries a header: one the preferred parser recognized
    /// structurally, or a license notice in another style.
    pub fn has_existing_header(&self) -> bool {
        self.header_detected() || self.looks_licensed()
    }

    /// Strip ALL header blocks in any listed non-preferred style, then move the insertion point
    /// to where the preferred style wants its header. Returns whether anything was removed.
    /// Unlike [`Self::remove_header`] (topmost-only), this drains every foreign block: a file
    /// may carry several, and a leftover would re-emit the preferred header on top of it.
    pub fn remove_foreign_header(&mut self) -> bool {
        // No removal candidates -> nothing foreign to strip. Return before cloning content.
        if self.removal_candidates.is_empty() {
            return false;
        }

        let filepath = self.filepath.to_string_lossy().to_string();
        let mut removed_any = false;

        // Re-snapshot after each deletion so the next probe's offsets stay valid. Each probe
        // removes a non-empty span, so the buffer shrinks and the loop terminates.
        loop {
            let content = self.parser.file_content.content().to_string();
            let mut removed_this_pass = false;
            for def in &self.removal_candidates {
                let probe = parse_header(
                    FileContent::from_content(content.clone(), filepath.clone()),
                    def,
                    &self.keywords,
                );
                if let Some(end) = probe.end_pos {
                    self.parser.file_content.delete(probe.begin_pos, end);
                    removed_any = true;
                    removed_this_pass = true;
                    break; // re-snapshot before probing again
                }
            }
            if !removed_this_pass {
                break;
            }
        }

        if removed_any {
            // Foreign skip handling may differ from the preferred style. Re-parse with the
            // preferred def so the insertion point lands where the preferred style wants it (not
            // at a foreign begin_pos) and any block exposed beneath the stripped one shows in
            // `end_pos` for the caller's strip loop to remove instead of stacking a duplicate.
            self.reparse_preferred(&filepath);
        }

        removed_any
    }

    /// Strip every header block (preferred plus all listed foreign styles) so exactly one
    /// preferred header can be inserted afterward. Returns whether anything was removed. The
    /// `format` replace path uses this to migrate a file stacking mixed styles, in either order.
    pub fn remove_existing_headers(&mut self) -> bool {
        let filepath = self.filepath.to_string_lossy().to_string();
        let mut removed_any = false;

        // Terminates: each continuing iteration deletes a non-empty span, so the buffer shrinks;
        // the final iteration removes nothing and breaks.
        loop {
            if self.header_detected() {
                self.remove_header();
                removed_any = true;
                // `remove_header` leaves `parser.end_pos` set, so re-parse to refresh detection
                // (and expose any block beneath the one just removed); else this loops forever.
                self.reparse_preferred(&filepath);
                continue;
            }
            if self.remove_foreign_header() {
                removed_any = true;
                continue;
            }
            break;
        }

        removed_any
    }

    /// Replace `self.parser` with a fresh parse of the current content against the preferred
    /// header def, so `begin_pos`/`end_pos` reflect what is left after edits.
    fn reparse_preferred(&mut self, filepath: &str) {
        let content = self.parser.file_content.content().to_string();
        self.parser = parse_header(
            FileContent::from_content(content, filepath.to_string()),
            &self.header_def,
            &self.keywords,
        );
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
        atomic_write(filepath, self.parser.file_content.content())
            .or_raise(|| Error::new(format!("cannot save document {}", filepath.display())))
    }

    /// Write to a throwaway `--dry-run` debug artifact (`.formatted`/`.removed`). Never the
    /// user's source, so a plain write is enough; crash-safe [`atomic_write`] is for [`Self::save`].
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

/// Whole-word search: `needle` matches only when the chars bracketing each occurrence are word
/// boundaries (not alphanumeric, not `_`; string start/end count). Keeps `copyright` from
/// matching inside `copyrightHolder`. Both args are expected lowercased by the caller.
fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    // `match_indices` yields offsets at char boundaries, so the surrounding slices stay UTF-8
    // valid and `chars().next_back()`/`.next()` are safe.
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    haystack.match_indices(needle).any(|(start, m)| {
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word(c));
        let after_ok = haystack[start + m.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word(c));
        before_ok && after_ok
    })
}

/// Write `content` to `path` atomically: temp file, flush, rename over the target. A crash or
/// `ENOSPC` mid-write leaves the original intact (the rename never runs). Permissions are
/// preserved; ownership/ACLs are not. A symlink at `path` is replaced by a regular file
/// carrying the link TARGET's mode (`fs::metadata` follows the link, `fs::rename` replaces it).
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;

    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("out");
    // Same directory as the target so the rename stays on one filesystem (atomic, no EXDEV).
    let tmp = dir.join(format!(".{file_name}.hawkeye-{}.tmp", std::process::id()));

    let result = (|| {
        let mut file = File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        match fs::metadata(path) {
            // Best-effort: a perm-copy failure must not abort an otherwise-successful write.
            Ok(meta) => {
                if let Err(e) = fs::set_permissions(&tmp, meta.permissions()) {
                    log::warn!(
                        "{}: cannot copy existing permissions ({e}); new file uses default mode",
                        path.display()
                    );
                }
            }
            // Absent target is normal (umask perms). A metadata error on an existing path means
            // we cannot preserve its mode; warn rather than silently downgrade.
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                log::warn!(
                    "{}: cannot read existing permissions ({e}); new file uses default mode",
                    path.display()
                );
            }
            Err(_) => {}
        }
        fs::rename(&tmp, path)?;
        // Best-effort fsync of the parent dir so the rename survives a crash, not just the data.
        // Not cfg-gated: opening a dir as a file just fails where unsupported and we skip it.
        if let Ok(dir_file) = File::open(dir) {
            let _ = dir_file.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    use crate::header::model::default_headers;

    use super::*;

    /// After removing a header in a foreign style whose skip handling differs from the
    /// preferred style, the insertion point must be re-derived with the preferred def. Here the
    /// foreign `javapkg_style` skips the leading `package ...;` line, but the preferred
    /// `doubleslash_style` has no skip pattern, so the new header belongs at the top of the
    /// file. Pins the bug where `begin_pos` kept the foreign (post-package) offset.
    #[test]
    fn foreign_skip_pattern_does_not_offset_preferred_insertion() {
        let defs = default_headers();
        let preferred = defs.get("doubleslash_style").unwrap().clone();
        let javapkg = defs.get("javapkg_style").unwrap().clone();

        let content =
            "package com.foo;\n\n/*\n * Copyright 2099 Old Authors.\n */\n\npub fn answer() -> i32 { 42 }\n";

        // Unique on-disk path: `Document::new` -> `FileContent::new` reads the file.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let fixture = std::env::temp_dir().join(format!(
            "hawkeye-foreign-skip-{}-{}.rs",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&fixture, content).unwrap();

        let mut doc = Document::new(
            fixture.clone(),
            preferred,
            vec![javapkg], // removal-only style that skips the `package` line
            vec!["copyright".to_string()],
            ExistingStrategy::Replace,
            HashMap::new(),
            Attributes {
                filename: fixture.file_name().map(|s| s.to_string_lossy().to_string()),
                disk_file_created_year: None,
                git_file_created_year: None,
                git_file_modified_year: None,
                git_authors: Default::default(),
            },
        )
        .unwrap()
        .unwrap();

        // The /* */ header is not in the preferred // style, so structural detection misses it.
        assert!(!doc.header_detected());
        assert!(doc.remove_foreign_header());
        // Preferred doubleslash has no skip pattern, so the header goes to the top of the file,
        // not after `package com.foo;` where the foreign javapkg probe stopped.
        assert_eq!(doc.parser.begin_pos, 0);

        let _ = fs::remove_file(&fixture);
    }

    /// Two stacked foreign-style blocks must BOTH be stripped, not just the first: leaving the
    /// second behind would re-emit the preferred header on top of a surviving notice (the #210
    /// stacking regression). Pins the loop in [`Document::remove_foreign_header`].
    #[test]
    fn remove_foreign_header_strips_all_stacked_blocks() {
        let defs = default_headers();
        let preferred = defs.get("doubleslash_style").unwrap().clone();
        let slashstar = defs.get("slashstar_style").unwrap().clone();

        let content = "/*\n * Copyright 2099 Vendor Authors.\n */\n\n/*\n * Copyright 2015 Older Project Authors.\n */\n\npub fn answer() -> i32 { 42 }\n";

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let fixture = std::env::temp_dir().join(format!(
            "hawkeye-stacked-foreign-{}-{}.rs",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&fixture, content).unwrap();

        let mut doc = Document::new(
            fixture.clone(),
            preferred,
            vec![slashstar], // /* */ accepted for removal only
            vec!["copyright".to_string()],
            ExistingStrategy::Replace,
            HashMap::new(),
            Attributes {
                filename: fixture.file_name().map(|s| s.to_string_lossy().to_string()),
                disk_file_created_year: None,
                git_file_created_year: None,
                git_file_modified_year: None,
                git_authors: Default::default(),
            },
        )
        .unwrap()
        .unwrap();

        assert!(!doc.header_detected());
        assert!(doc.remove_foreign_header());
        // Both blocks gone: no keyword left and no /* */ comment survives.
        assert!(!doc.looks_licensed());
        assert!(!doc.parser.file_content.content().contains("/*"));

        let _ = fs::remove_file(&fixture);
    }

    /// A notice pushed down by many leading lines must still be detected: [`Document::looks_licensed`]
    /// scans a bounded character prefix, not a fixed line count, so a notice deep in the prefix
    /// (here after 60 filler lines) is still found.
    #[test]
    fn looks_licensed_detects_notice_after_many_leading_lines() {
        let defs = default_headers();
        let preferred = defs.get("doubleslash_style").unwrap().clone();

        let mut content = String::new();
        for i in 0..60 {
            content.push_str(&format!("// filler line {i}\n"));
        }
        content.push_str("// Copyright 2099 Late Authors.\n\npub fn answer() -> i32 { 42 }\n");

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let fixture = std::env::temp_dir().join(format!(
            "hawkeye-late-notice-{}-{}.rs",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&fixture, &content).unwrap();

        let doc = Document::new(
            fixture.clone(),
            preferred,
            vec![],
            vec!["copyright".to_string()],
            ExistingStrategy::Replace,
            HashMap::new(),
            Attributes {
                filename: fixture.file_name().map(|s| s.to_string_lossy().to_string()),
                disk_file_created_year: None,
                git_file_created_year: None,
                git_file_modified_year: None,
                git_authors: Default::default(),
            },
        )
        .unwrap()
        .unwrap();

        assert!(doc.looks_licensed());

        let _ = fs::remove_file(&fixture);
    }

    /// Build a `Document` over `content` (written to a unique on-disk path) with `doubleslash`
    /// preferred and `slashstar` accepted for removal only.
    fn replace_document(tag: &str, content: &str) -> (Document, PathBuf) {
        let defs = default_headers();
        let preferred = defs.get("doubleslash_style").unwrap().clone();
        let slashstar = defs.get("slashstar_style").unwrap().clone();

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let fixture = std::env::temp_dir().join(format!(
            "hawkeye-{tag}-{}-{}.rs",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&fixture, content).unwrap();

        let doc = Document::new(
            fixture.clone(),
            preferred,
            vec![slashstar],
            vec!["copyright".to_string()],
            ExistingStrategy::Replace,
            HashMap::new(),
            Attributes {
                filename: fixture.file_name().map(|s| s.to_string_lossy().to_string()),
                disk_file_created_year: None,
                git_file_created_year: None,
                git_file_modified_year: None,
                git_authors: Default::default(),
            },
        )
        .unwrap()
        .unwrap();
        (doc, fixture)
    }

    /// Run the `format` replace path (strip-all then insert one preferred header) and assert
    /// exactly one resulting header and no surviving old block, regardless of stacking order.
    /// Mixing a preferred-style block with a listed-foreign-style block must not double-notice
    /// (the #210 regression for mixed-style files, both orders).
    fn assert_single_header_after_replace(doc: &mut Document) {
        let header = HeaderMatcher::new("Copyright 2024 New Authors.\n".to_string());
        assert!(doc.remove_existing_headers());
        doc.update_header(&header).unwrap();

        let result = doc.parser.file_content.content();
        assert_eq!(
            result.matches("Copyright").count(),
            1,
            "exactly one header must remain, got: {result:?}"
        );
        // No old notice and no foreign /* */ block survives.
        assert!(
            !result.contains("Old Authors"),
            "stale block survived: {result:?}"
        );
        assert!(!result.contains("/*"), "foreign block survived: {result:?}");
    }

    #[test]
    fn replace_strips_foreign_block_above_preferred_block() {
        let content = "/*\n * Copyright 2099 Old Authors (foreign style).\n */\n\n// Copyright 2015 Old Authors (preferred style).\n\npub fn answer() -> i32 { 42 }\n";
        let (mut doc, fixture) = replace_document("d2-foreign-above", content);
        assert_single_header_after_replace(&mut doc);
        let _ = fs::remove_file(&fixture);
    }

    #[test]
    fn replace_strips_preferred_block_above_foreign_block() {
        let content = "// Copyright 2015 Old Authors (preferred style).\n\n/*\n * Copyright 2099 Old Authors (foreign style).\n */\n\npub fn answer() -> i32 { 42 }\n";
        let (mut doc, fixture) = replace_document("d2-preferred-above", content);
        assert_single_header_after_replace(&mut doc);
        let _ = fs::remove_file(&fixture);
    }

    /// Form-1 residual-duplicate edge (#210): a listed-style block ABOVE a notice in an UNLISTED
    /// style. Stripping the listed block leaves the unlisted notice behind, so the file STILL
    /// looks licensed afterward. The `format` replace path relies on this post-strip signal to
    /// route the file to `foreign` instead of inserting a preferred header on top of the
    /// survivor (which would re-stack the duplicate and never self-heal).
    #[test]
    fn replace_residual_unlisted_notice_still_looks_licensed_after_strip() {
        // slashstar (listed, removed) on top; a hash-comment notice (unlisted) below.
        let content = "/*\n * Copyright 2099 Vendor (listed slashstar style).\n */\n\n# Copyright 2015 Project (unlisted hash style).\n\npub fn answer() -> i32 { 42 }\n";
        let (mut doc, fixture) = replace_document("residual-unlisted", content);

        assert!(
            doc.remove_existing_headers(),
            "the listed slashstar block must be removed"
        );
        // The unlisted hash notice cannot be stripped, so it survives and the file still looks
        // licensed: the replace path must re-check this and route to foreign, not insert on top.
        assert!(doc.looks_licensed());
        assert!(doc.parser.file_content.content().contains('#'));

        let _ = fs::remove_file(&fixture);
    }

    /// Build a single-style (`doubleslash`) `Document` over `content`, so `looks_licensed` is
    /// the only existing-header signal (no removal candidates to confuse the probe).
    fn single_style_document(tag: &str, content: &str) -> (Document, PathBuf) {
        let defs = default_headers();
        let preferred = defs.get("doubleslash_style").unwrap().clone();

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let fixture = std::env::temp_dir().join(format!(
            "hawkeye-{tag}-{}-{}.rs",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&fixture, content).unwrap();

        let doc = Document::new(
            fixture.clone(),
            preferred,
            vec![],
            vec!["copyright".to_string()],
            ExistingStrategy::Replace,
            HashMap::new(),
            Attributes {
                filename: fixture.file_name().map(|s| s.to_string_lossy().to_string()),
                disk_file_created_year: None,
                git_file_created_year: None,
                git_file_modified_year: None,
                git_authors: Default::default(),
            },
        )
        .unwrap()
        .unwrap();
        (doc, fixture)
    }

    /// Word-boundary fix: `copyright` embedded in the identifier `copyrightHolder` must NOT mark
    /// the file as licensed, or `format` would wrongly refuse to insert a header.
    #[test]
    fn looks_licensed_ignores_keyword_inside_identifier() {
        let content = "struct Config { copyrightHolder: String }\n";
        let (doc, fixture) = single_style_document("word-ident", content);
        assert!(!doc.looks_licensed());
        let _ = fs::remove_file(&fixture);
    }

    /// A standalone `Copyright` word (here in a `/* */` block) is a real notice and must match,
    /// even though the surrounding comment style is not the preferred one.
    #[test]
    fn looks_licensed_matches_standalone_word_in_comment() {
        let content = "/*\n * Copyright 2099 Real Authors.\n */\n\npub fn answer() -> i32 { 42 }\n";
        let (doc, fixture) = single_style_document("word-comment", content);
        assert!(doc.looks_licensed());
        let _ = fs::remove_file(&fixture);
    }

    /// A whole-word keyword inside a string literal still classifies as foreign by design: we
    /// prefer a controlled failure over silently inserting a second notice.
    #[test]
    fn looks_licensed_matches_standalone_word_in_string_literal() {
        let content = "let s = \"copyright 2020\";\n";
        let (doc, fixture) = single_style_document("word-string", content);
        assert!(doc.looks_licensed());
        let _ = fs::remove_file(&fixture);
    }

    /// Unit coverage for the boundary logic in isolation, including start/end-of-string and the
    /// `_` word-char rule.
    #[test]
    fn contains_whole_word_boundaries() {
        assert!(contains_whole_word("copyright 2024", "copyright"));
        assert!(contains_whole_word("a copyright.", "copyright"));
        assert!(contains_whole_word("copyright", "copyright")); // whole string
        assert!(!contains_whole_word("copyrightholder", "copyright"));
        assert!(!contains_whole_word("my_copyright", "copyright")); // `_` is a word char
        assert!(!contains_whole_word("copyright_owner", "copyright"));
        assert!(!contains_whole_word("anything", ""));
    }

    /// [`atomic_write`] over a SYMLINK must replace the link with a regular file carrying the
    /// pre-existing target's permission bits, leave the original target untouched, and leave no
    /// `.tmp` behind. Pins the symlink-replacement and permission-preservation contract.
    #[cfg(unix)]
    #[test]
    fn atomic_write_replaces_symlink_with_regular_file() {
        use std::os::unix::fs::PermissionsExt;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "hawkeye-atomic-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();

        let target = dir.join("target.rs");
        let link = dir.join("link.rs");
        fs::write(&target, "old content\n").unwrap();
        // A non-default mode so the assertion is meaningful (not whatever umask produces).
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        atomic_write(&link, "new content\n").unwrap();

        // The link path is now a regular file (rename replaced the link, not its target).
        let link_meta = fs::symlink_metadata(&link).unwrap();
        assert!(link_meta.file_type().is_file());
        assert!(!link_meta.file_type().is_symlink());
        assert_eq!(fs::read_to_string(&link).unwrap(), "new content\n");

        // Original target is untouched: rename replaced the link, not what it pointed at.
        assert_eq!(fs::read_to_string(&target).unwrap(), "old content\n");

        // New file inherited the target's mode bits (metadata followed the link to read them).
        assert_eq!(link_meta.permissions().mode() & 0o777, 0o640);

        // No leftover temp file in the directory.
        let leftover = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".hawkeye-"));
        assert!(!leftover, "a .hawkeye-*.tmp file was left behind");

        let _ = fs::remove_dir_all(&dir);
    }
}
