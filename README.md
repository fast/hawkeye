# HawkEye

[![Crates.io][crates-badge]][crates-url]
[![Documentation][docs-badge]][docs-url]
[![MSRV 1.88][msrv-badge]](https://www.whatrustisit.com)
[![Apache 2.0 licensed][license-badge]][license-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/hawkeye.svg
[crates-url]: https://crates.io/crates/hawkeye
[docs-badge]: https://img.shields.io/docsrs/hawkeye
[docs-url]: https://docs.rs/hawkeye
[msrv-badge]: https://img.shields.io/badge/MSRV-1.88-green?logo=rust
[license-badge]: https://img.shields.io/crates/l/hawkeye
[license-url]: https://www.apache.org/licenses/LICENSE-2.0
[actions-badge]: https://github.com/korandoru/hawkeye/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/korandoru/hawkeye/actions/workflows/ci.yml

HawkEye checks, formats, and removes source-file license headers. The package in `hawkeye` publishes both the reusable `hawkeye` library and the `hawkeye` command-line binary; the repository root is a virtual Cargo workspace.

HawkEye v7 deliberately uses a new snake-case configuration contract. It does not accept v6 field aliases; migration tooling belongs in a separate tool rather than the runtime parser.

## Installation

The v7 prerelease is distributed through crates.io and includes both the `hawkeye` library and command-line binary:

```shell
cargo install hawkeye --version 7.0.0-alpha.1 --locked
```

## Command line

Unless `--config` is passed, HawkEye tries `licenserc.toml` and then `.licenserc.toml` in the current directory. It does not search parent directories.

```shell
# Report non-canonical files.
hawkeye check

# Show what format would change without writing.
hawkeye check --diff

# Apply safe additions and replacements.
hawkeye format --fail-if-updated=false

# Remove structurally recognized headers.
hawkeye remove --fail-if-updated=false

# Emit the stable data shape without a separate report version.
hawkeye check --output json

# Inspect file discovery, Git commands, and timing.
RUST_LOG=hawkeye=debug hawkeye check
```

`check` exits with code 1 for a missing, non-canonical, or conflicting header. `format` and `remove` write safe changes first and then exit with code 1 by default if anything changed; pass `--fail-if-updated=false` for auto-fix workflows. All commands accept `--fail-if-unknown` to treat files without a rule and non-UTF-8 files as policy failures. Configuration, I/O, template, and Git failures use exit code 2.

`--dry-run` suppresses writes, while `--diff` prints unified diffs. JSON output is written to stdout; when JSON and diff output are requested together, diffs go to stderr so stdout remains valid JSON.

## Configuration

This example shows every configuration section. Exactly one of `header.builtin`, `header.path`, or `header.text` is allowed.

```toml
[header]
builtin = "Apache-2.0"
keywords = ["copyright"]

[files]
root = "."
includes = ["**/*.rs", "**/*.toml"]
excludes = ["generated/**"]

[props]
copyright_owner = "Acme Developers"
inception_year = 2026

[git]
ignore = "auto"
file_attrs = "disable"

[styles.quoted_line]
kind = "line"
prefix = "<!-- "
suffix = " -->"
pad_lines = true

[styles.quoted_block]
kind = "block"
start = "<!--"
prefix = "    "
suffix = ""
end = "-->"

[[rules]]
extensions = ["rs", "d.ts"]
filenames = ["Cargo.toml"]
style_out = "slash_line"
styles_in = ["slash_block"]
```

### Header

`header.builtin` is an opaque, case-sensitive resource key. v7 currently ships `Apache-2.0`, `Apache-2.0-ASF`, and `Elastic-2.0`; SPDX-like spelling is preserved instead of normalized to a Rust identifier.

`header.path` loads a UTF-8 MiniJinja template. Relative paths use the directory containing `licenserc.toml`; absolute paths are accepted. `header.text` stores the same template inline.

`header.keywords` defaults to `["copyright"]`. Every keyword must occur case-insensitively in a structurally parsed header before HawkEye may replace or remove it. This small semantic gate prevents an ordinary leading comment that happens to use the same comment syntax from becoming a deletion range.

### Template context

Templates receive two top-level objects: user-defined `props` and per-file `attrs`. MiniJinja runs with strict undefined values, no auto-escaping, and its standard built-in filters, tests, and functions. HawkEye does not register filesystem, process, network, dynamic loader, or include capabilities.

`props` contains the values from the `[props]` TOML table. Built-in Apache and Elastic templates use `props.inception_year` and `props.copyright_owner`.

`attrs` contains:

- `filename`: the file basename;
- `disk_file_created_year`: filesystem creation year or `null` when unavailable;
- `disk_file_modified_year`: filesystem modification year or `null` when unavailable;
- `git_file_created_year`: earliest non-merge commit year for the current path or `null` when Git attributes are disabled;
- `git_file_modified_year`: latest non-merge commit year, upgraded to the current year for a dirty or untracked file;
- `git_authors`: sorted distinct Git author names, including the configured current user for dirty or untracked files.

Unavailable values remain `null`; they are never silently replaced with the current year. A template can choose its own fallback explicitly.

### File discovery

`files.root` defaults to the directory containing `licenserc.toml`. Relative roots use that directory; absolute roots are accepted.

`files.includes` and `files.excludes` are Git-ignore-style path filters relative to `files.root`, with `/` as the logical separator. An empty `includes` list means all files, after which excludes and rule selection still apply. `.git` is always excluded. HawkEye does not maintain another built-in list of generated or dependency directories.

`git.ignore` is `disable`, `auto`, or `enable` and defaults to `auto`. When a repository is available, HawkEye asks Git for tracked and non-ignored untracked files. This preserves Git's index semantics: a file force-added with `git add -f` remains selected even if it also matches `.gitignore`. Outside a repository, `auto` falls back to an ordinary filesystem walk because `.gitignore` has no repository context; `enable` requires `files.root` to be inside a Git worktree.

### Rules and styles

Rules are checked in declaration order, followed by HawkEye's built-in language rules as low-priority fallbacks. `extensions` are exact case-insensitive suffixes without a leading dot, so `d.ts` directly supports a multi-segment extension. `filenames` are complete case-insensitive basenames. Rules do not use path globs.

`style_out` is the one canonical output syntax. `styles_in` adds syntaxes that can be structurally recognized and safely replaced or removed. The output style is always accepted as input and need not be repeated. If the leading text parses as a known comment header and contains all configured keywords but its style is not accepted by the rule, HawkEye reports `conflict` instead of guessing a deletion range.

Custom line styles wrap each logical header line with `prefix` and `suffix`. `pad_lines = true` right-pads shorter lines so suffixes align; it requires a non-empty suffix. Custom block styles write `start` and `end` on their own lines and wrap body lines with `prefix` and `suffix`.

Built-in output styles include line comments for slash, hash, dash, percent, semicolon, apostrophe, bang, tilde, batch, and Haml syntaxes, plus block comments for C, XML, Lua, Pascal, Velocity, Mustache, MVEL, FreeMarker, JSP, ColdFusion, ASP, Swift banners, and AsciiDoc. The built-in filename and extension rules cover the corresponding v6 language set, while user rules always take precedence.

### Git file attributes

`git.file_attrs` is `disable`, `auto`, or `enable` and defaults to `disable` because history traversal has a cost. `auto` populates attributes when a repository is available; `enable` turns repository discovery and Git command failures into operational errors.

History is traversed once per run. Each non-merge commit is compared through Git's normal changed-path output, avoiding a merge commit being attributed as a file modification merely because histories joined. Dirty tracked files and files inside untracked directories use the current UTC year and current configured Git author.

## Library

The library exposes the same behavior without shelling out to the CLI:

```rust
use hawkeye::{Engine, Mode};

let engine = Engine::load("licenserc.toml")?;
let plan = engine.plan(Mode::Check)?;
let report = plan.report();
# Ok::<(), hawkeye::Error>(())
```

`Config` is the Serde-facing TOML model. `Config::resolve` produces `ResolvedConfig`, which owns resolved paths, compiled templates, styles, and ordered rules. `Engine::plan` performs discovery and analysis without writes; `Plan::apply` performs atomic same-directory replacements after checking that each input is unchanged. Symbolic links and multiply linked files are not replaced.

## Development

Repository workflows are exposed through `cargo x`:

```shell
cargo x build
cargo x test
cargo x lint
```

Releases use `cargo-release` directly. `cargo release 7.0.0-alpha.1` previews the Alpha 1 release, and adding `--execute` performs the configured commit, crates.io publish, signed tag, and push.

The virtual workspace keeps the product, integration corpus, and development tasks separate without introducing a `crates` directory for a single published package:

- `hawkeye` is the only published package and contains the library and command-line binary;
- `tests-integration` is an unpublished package containing complete repository corpora and Rust-driven integration tests;
- `xtask` is an unpublished development tool.

Integration tests copy each corpus to a temporary directory, optionally create a real Git repository and history, snapshot the initial tree and reports, run `format`, snapshot the resulting tree, and verify that subsequent `check` and `format` runs are clean and idempotent. The root `licenserc.toml` excludes the entire `tests-integration` directory because those corpora intentionally contain missing, legacy, conflicting, ignored, BOM, CRLF, and otherwise non-canonical files.

## Minimum Rust version policy

The minimum supported Rust version is 1.88.0.

The minimum supported Rust version may be increased in a minor release. Patch releases will preserve the minimum supported Rust version of their corresponding minor release.

## License

This project is licensed under [Apache License, Version 2.0][license-url].
