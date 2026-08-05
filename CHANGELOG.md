# Changelog

## 7.0.0-alpha.1 - Unreleased

- Rebuild HawkEye as one `hawkeye` package that publishes both a library and a binary.
- Replace callback-driven formatting with resolved configuration, pure analysis, explicit byte-range edits, safe filesystem application, and deterministic structured reports.
- Introduce ordered first-match language rules, distinct write/read styles, built-in comment styles, custom styles, and strict one-time MiniJinja header rendering.
- Add conflict-aware migration so an existing header in another comment style is never duplicated and may be normalized when that style is explicitly accepted.
- Preserve BOMs, shebangs, hash-language encoding/magic comments, XML/PHP preambles, line endings, permissions, and unrelated source bytes while rejecting stale plans and symlink replacement.
- Provide `check`, `format`, and `remove` with human/JSON reports, unified diffs, dry runs, and stable exit-code categories.
- Add a library-only feature profile, MSRV validation, cross-platform CI, dependency policy checks, real CLI tests, regression coverage, and a representative downstream corpus.
- Remove the v6 configuration format, composite GitHub Action, Docker distribution, Python integration harness, Git-history-derived template attributes, and multi-crate workspace.
