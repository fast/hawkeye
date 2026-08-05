# HawkEye v7 design

## Objective

HawkEye v7 is a clean implementation of a license-header analysis and rewrite engine. It is one published Cargo package with a reusable library and a thin command-line binary. Compatibility with v6 code, configuration, output, and distribution forms is not a goal.

## Architecture

The data flow is unidirectional:

```text
Config -> ResolvedConfig -> discovery -> analysis -> EditPlan -> apply/report
```

Configuration parsing and semantic resolution are separate. Analysis consumes in-memory content and cannot perform filesystem writes. An edit contains an explicit byte range, the expected original bytes, and replacement text. Filesystem application is the only layer allowed to mutate source files. Reports are structured library values; the CLI only renders them and maps them to exit codes.

The library does not log, terminate the process, hide edits behind callbacks, or panic for user-controlled input.

## File states

Every selected file resolves to exactly one state:

| State | Meaning | Check | Format | Remove |
| --- | --- | --- | --- | --- |
| `clean` | The preferred header has the expected content. | Pass | No change | Remove the proven range |
| `missing` | No license-header candidate exists. | Finding | Add the preferred header | No change |
| `replaceable` | A recognized header has a proven range but stale content or a non-preferred accepted style. | Finding | Replace the proven range | Remove the proven range |
| `conflict` | The prefix looks licensed, but no safe interpretation is available. | Finding | No change | No change |
| `unsupported` | No rule applies, or the file cannot be analyzed as supported UTF-8 text. | Report | No change | No change |

Normal `format` and `remove` runs apply safe edits even when another file conflicts. Conflicting files remain byte-for-byte unchanged and make the process exit with code `1`.

## Header recognition

Recognition only examines structurally parsed comment blocks at the legal header position after a supported preamble. Source-code string contents are never treated as headers.

A comment block is proven to be the configured header when either its whitespace-normalized body equals the rendered template or all configured identifiers occur case-insensitively and its line count equals the rendered template. The latter deliberately supports changing years and other same-shape template values. Identifiers should therefore be specific, repository-controlled phrases rather than generic words.

An identified block with a different shape, an identified header in an unlisted style, or a license header hidden behind another leading comment becomes `conflict`. This conservative state prevents the duplicate-header failure described in issue #210 without guessing at a destructive edit.

Consecutive recognized headers in allowed styles form one proven range. Formatting collapses that range to one preferred header, which makes style migrations and cleanup idempotent.

## Safety invariants

- Deletion and replacement require a structurally parsed comment range and configured identity evidence.
- Free-form keyword sightings outside an allowed structural candidate may only produce `conflict`; they never authorize a destructive edit.
- A conflict leaves the original bytes unchanged.
- Formatting is idempotent.
- Formatting a safely analyzable file produces exactly one recognized license header.
- Analysis preserves unrelated bytes, line endings, byte-order marks, shebangs, XML declarations, and PHP opening preambles.
- A planned edit captures its expected input, and repository application rejects files changed after planning.
- Discovery does not follow symlinks, and the write layer refuses to replace a symlink if one reaches it.
- Replacements use a synchronized temporary file in the target directory, preserve permissions, and persist by atomic rename.

## Configuration contract

The default filename is `hawkeye.toml`. Every key uses `snake_case`, unknown fields are rejected, and relative paths resolve only against the configuration directory.

`[header]` selects exactly one inline `text` or external `path`. `[variables]` supplies MiniJinja values and is rendered once per engine load with strict undefined-variable behavior. `[files]` controls Git ignore support and explicit include/exclude globs.

Ordered `[[rules]]` use repository-relative path globs with first-match precedence. User rules precede built-in defaults. Each rule has one `write_style` and zero or more additional `read_styles`; the write style is always included in the readable set. Read styles describe structural recognition and migration, not command policy.

A representative configuration is:

```toml
[header]
path = "headers/Apache-2.0-ASF.txt"
identifiers = ["Apache Software Foundation", "Apache License, Version 2.0"]

[variables]
year = 2026

[files]
use_gitignore = true
include = ["src/**", "tests/**"]
exclude = ["**/generated/**", "**/*.lock"]

[[rules]]
patterns = ["*.rs", "**/*.rs", "*.go", "**/*.go"]
write_style = "slash"
read_styles = ["slash_star"]
```

Built-in styles cover `//`, `#`, `--`, C-style blocks, and XML comments. Custom styles use an internally tagged `kind = "line"` or `kind = "block"` schema. Setting `use_default_rules = false` disables the built-in language mappings and requires at least one explicit rule.

## Discovery and reports

The configuration directory is the repository root for one engine run. Discovery includes hidden and untracked files, honors Git ignore data by default, always skips `.git`, never follows links, omits the active configuration and external header source, and sorts paths deterministically.

Reports retain native `PathBuf` values in the library. Serialization emits paths as Unicode strings using lossy conversion for non-Unicode operating-system paths so one unusual filename cannot corrupt otherwise valid JSON output.

## CLI contract

The command surface is `check`, `format`, and `remove`. `check --diff` shows the safe changes that `format` would make. `format` and `remove` support `--dry-run` and `--diff`. Human output and JSON reports are both written to stdout; diagnostics are written to stderr. Unified diff and JSON output are mutually exclusive so stdout always has one parseable format.

Exit codes are reserved as follows:

- `0`: the command completed and its policy was satisfied.
- `1`: analysis findings, pending dry-run changes, or semantic conflicts.
- `2`: invalid invocation, configuration or template failure, discovery failure, or operational I/O failure.

## Deliberately deferred

The first v7 release does not include legacy-config migration, the composite GitHub Action, Docker distribution, Git-history-derived template attributes, tree-sitter parsers, parallel execution, or a plugin system. Git ignore support remains part of file discovery and is independent from Git history analysis.

## Delivery sequence

1. Establish the single-package skeleton and contracts.
2. Implement resolved configuration, rules, styles, pure analysis, and edit planning.
3. Implement discovery, Git ignore handling, filesystem application, and reports.
4. Implement the complete CLI and output contract.
5. Add regression fixtures, behavioral invariants, cross-platform coverage, and downstream corpus checks.
6. Validate packaging and publish a v7 alpha.

The v6 implementation remains available in the dedicated snapshot worktree and branch for behavioral reference, not as code to migrate incrementally.
