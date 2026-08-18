# HawkEye

[![Crates.io][crates-badge]][crates-url]
[![Documentation][docs-badge]][docs-url]
[![MSRV 1.89][msrv-badge]](https://www.whatrustisit.com)
[![Apache 2.0 licensed][license-badge]][license-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/hawkeye.svg
[crates-url]: https://crates.io/crates/hawkeye
[docs-badge]: https://img.shields.io/docsrs/hawkeye
[docs-url]: https://docs.rs/hawkeye
[msrv-badge]: https://img.shields.io/badge/MSRV-1.89-green?logo=rust
[license-badge]: https://img.shields.io/crates/l/hawkeye
[license-url]: https://www.apache.org/licenses/LICENSE-2.0
[actions-badge]: https://github.com/korandoru/hawkeye/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/korandoru/hawkeye/actions/workflows/ci.yml

HawkEye checks, formats, and removes source-file license headers. The crate provides both the `hawkeye` command-line tool and a Rust library.

## Installation

The recommended way to install the command-line tool is to let [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) download a prebuilt release:

```shell
cargo binstall hawkeye@7.0.0-alpha.1
```

To build from source instead, use Cargo:

```shell
cargo install hawkeye --version 7.0.0-alpha.1 --locked
```

Prebuilt releases cover the following platforms:

| Distribution   | Platforms                                                                                               |
| -------------- | ------------------------------------------------------------------------------------------------------- |
| cargo-binstall | macOS and Linux on x86-64 or ARM64, plus Windows on x86-64; Linux archives support both glibc and musl. |
| Docker image   | Linux on amd64 or arm64.                                                                                |

## Getting started

Add `licenserc.toml` to the project root. This minimal configuration uses the bundled Apache 2.0 header and the built-in file rules:

```toml
[header]
builtin = "Apache-2.0"

[props]
copyright_owner = "Acme Developers"
inception_year = 2026
```

Then check or update the project:

```shell
hawkeye check
hawkeye format
```

## Command line

| Command          | Behavior                                                                       |
| ---------------- | ------------------------------------------------------------------------------ |
| `hawkeye check`  | Reports missing, non-canonical, and conflicting headers without writing files. |
| `hawkeye format` | Adds missing headers and replaces recognized non-canonical headers.            |
| `hawkeye remove` | Removes recognized headers.                                                    |

Pass files or directories after a command to avoid scanning the rest of a large repository. `--files-from` reads newline- or NUL-separated paths from a file, and `-` reads stdin:

```shell
hawkeye check src/lib.rs src/bin
git diff --name-only -z origin/main | hawkeye check --files-from -
```

Command-line paths are resolved from the current directory. They still obey `files.root`, `files.includes`, and `files.excludes`; an explicitly named file bypasses Git ignore rules, while a named directory uses normal discovery. Missing paths are skipped with a warning, while paths outside `files.root` are ignored as out of scope. An explicitly supplied empty list selects no files.

Without `--config`, HawkEye tries `licenserc.toml` and then `.licenserc.toml` in the current directory. It does not search parent directories.

All commands support `--output-format json` and `--fail-on-unknown`. `format` and `remove` also support `--dry-run` and `--fail-on-change`. Reports go to stdout; logs and errors go to stderr. Set `RUST_LOG=hawkeye=debug` to inspect file discovery and Git processing.

Exit code 0 means the selected policy passed. Exit code 1 means `check` found a required change or conflict, an edit command left a conflict, or an enabled failure option matched. Config, I/O, template, and Git errors use exit code 2.

## Integrations

### Docker

The distroless image runs HawkEye in `/workspace`. Pass the host user when formatting a bind mount so that writes retain the expected ownership:

```shell
docker run --rm --user "$(id -u):$(id -g)" --volume "$PWD:/workspace" ghcr.io/korandoru/hawkeye:v7.0.0-alpha.1 check
```

### GitHub Actions

Install the released binary with cargo-binstall and invoke HawkEye directly; no HawkEye-specific action is required:

```yaml
- uses: actions/checkout@v7
- uses: taiki-e/install-action@v2
  with:
    tool: cargo-binstall
- run: cargo binstall hawkeye@7.0.0-alpha.1 --no-confirm
- run: hawkeye check
```

### pre-commit

The default hook installs the matching HawkEye source revision in pre-commit's isolated Python environment:

```yaml
repos:
  - repo: https://github.com/korandoru/hawkeye
    rev: v7.0.0-alpha.1
    hooks:
      - id: hawkeye-format
```

The hooks pass only the files selected by pre-commit. The Python hook needs a Rust toolchain when its environment is created for the first time. Use `hawkeye-format-docker` instead when Docker is the preferred runtime.

## Configuration

The following example shows every configuration section. Field names are snake case and unknown fields are rejected.

```toml
[header]
# Choose exactly one source. Built-in keys are case-sensitive.
builtin = "Apache-2.0"
# path = "HEADER.txt"
# text = "Copyright {{ props.inception_year }} {{ props.copyright_owner }}"

# Every keyword must occur, case-insensitively, before an existing comment can
# be replaced or removed. The default is ["copyright"].
keywords = ["copyright"]

[files]
# Relative paths are resolved from the directory containing this config file.
root = "."
# An empty includes list selects every discovered file.
includes = ["**/*.rs", "**/*.toml"]
excludes = ["generated/**"]

[props]
# Arbitrary TOML values are exposed to the header template as `props`.
copyright_owner = "Acme Developers"
inception_year = 2026

[git]
# Both fields accept "disable", "auto", or "enable".
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
# User rules are matched in declaration order before built-in rules.
extensions = ["rs", "d.ts"]
filenames = ["Cargo.toml"]
# format writes one canonical style.
style_out = "doubleslash"
# An empty list accepts only style_out; otherwise list every accepted style.
styles_in = ["doubleslash", "slashstar"]
```

### Headers and templates

`header.builtin` accepts `Apache-2.0`, `Apache-2.0-ASF`, or `Elastic-2.0`. `header.path` loads a UTF-8 template, while `header.text` stores the same template inline. A template file is never selected as a source file.

Templates use MiniJinja with strict undefined values, auto-escaping disabled, and its standard built-in filters, tests, and functions. HawkEye does not enable template includes or external loaders. The template context contains:

| Value                           | Meaning                                                                         |
| ------------------------------- | ------------------------------------------------------------------------------- |
| `props`                         | The complete user-defined `[props]` table.                                      |
| `attrs.filename`                | The current file name.                                                          |
| `attrs.disk_file_created_year`  | The filesystem creation year, or `null` when unavailable.                       |
| `attrs.disk_file_modified_year` | The filesystem modification year, or `null` when unavailable.                   |
| `attrs.git_file_created_year`   | The first Git commit year for the path, or `null` when disabled or unavailable. |
| `attrs.git_file_modified_year`  | The last Git commit year, using the current year for dirty or untracked files.  |
| `attrs.git_authors`             | Sorted distinct Git author names.                                               |

HawkEye never substitutes the current year for an unavailable value. Templates that need a fallback must express it explicitly.

### Files, rules, and styles

`files.includes` and `files.excludes` use Git-ignore-style patterns relative to `files.root`; negated patterns are not accepted because inclusion and exclusion are separate lists. An empty `includes` list means all discovered files. `.git` is always excluded. File symlinks are followed, while directory symlinks are not traversed.

Rules match complete filenames or case-insensitive filename suffixes. Extensions omit the leading dot and may contain multiple segments, such as `d.ts`. The first matching user rule wins; built-in rules are lower-priority fallbacks. Duplicate selectors are allowed and logged at debug level when shadowed.

`style_out` is the canonical format written by HawkEye. `styles_in` lists formats that may be recognized and safely replaced or removed. When `styles_in` is empty, it defaults to `[style_out]`; a non-empty list must include `style_out`. Style names are case-sensitive. A custom style may override a built-in style and produces a warning. The bundled mappings are defined in [rules.toml](hawkeye/src/builtin/rules.toml) and [styles.toml](hawkeye/src/builtin/styles.toml).

### Git integration

`git.ignore` defaults to `auto`: it uses the Git index and ignore rules inside a worktree and falls back to filesystem discovery outside one. Tracked files remain selected even when they match an ignore rule. Set the mode to `enable` to require a worktree or `disable` to use filesystem discovery unconditionally.

`git.file_attrs` defaults to `disable` because walking repository history has a cost. `auto` populates attributes when complete history is available; `enable` also requires a usable repository and complete history. HawkEye reads Git repositories in-process and does not require a `git` executable at runtime.

## Library

The library exposes the same engine used by the command-line tool:

```rust
use hawkeye::Config;
use hawkeye::Engine;

let config = Config::load("licenserc.toml")?;
let engine = Engine::new(config)?;
let report = engine.check()?;
# Ok::<(), hawkeye::Error>(())
```

`Engine::check` never writes files. `Engine::format` and `Engine::remove` return pending `Edits`; call `Edits::apply` to write them or `Edits::into_report` to inspect the result without writing. The corresponding `check_paths`, `format_paths`, and `remove_paths` methods accept strings, `Path`s, or `PathBuf`s and process only requested files and directories.

The default `application` feature builds the command-line tool. Library-only users can omit its command-specific dependencies:

```toml
hawkeye = { version = "7.0.0-alpha.1", default-features = false }
```

## Compatibility

HawkEye v7 uses a new snake-case configuration format and does not accept v6 field names. A v6 config must be migrated before use with v7.

The minimum supported Rust version is 1.89.0. It may be raised in a minor release; patch releases preserve the minimum version of their corresponding minor release.

## License

Licensed under the [Apache License, Version 2.0][license-url].
