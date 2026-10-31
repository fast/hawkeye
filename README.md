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
[actions-badge]: https://github.com/fast/hawkeye/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/fast/hawkeye/actions/workflows/ci.yml

HawkEye checks, formats, and removes source-file license headers. The crate provides both the `hawkeye` command-line tool and a Rust library.

## Installation

The recommended way to install the command-line tool is to let [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) download a prebuilt release:

```shell
cargo binstall hawkeye
```

To build from source instead, use Cargo:

```shell
cargo install hawkeye --locked
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

Pass files or directories after a command to avoid scanning the rest of a large repository:

```shell
hawkeye check src/lib.rs src/bin
git diff --name-only --diff-filter=ACMRT -z origin/main -- | xargs -0 -r hawkeye check --
```

Command-line paths are resolved from the current directory. They still obey `files.root`, `files.includes`, and `files.excludes`; an explicitly named file bypasses Git ignore rules, while a named directory uses normal discovery. Missing paths are skipped with a warning, while paths outside `files.root` are ignored as out of scope.

The `git diff` pipeline uses NUL-delimited paths so that unusual file names remain intact. Its `-r` option avoids running HawkEye with no arguments—and therefore avoids a full scan—when the diff is empty; the final `--` keeps file names from being parsed as HawkEye options.

Without `--config`, HawkEye tries `licenserc.toml` and then `.licenserc.toml` in the current directory. It does not search parent directories.

All commands support `--output-format json` and `--fail-on-unknown`. `format` and `remove` also support `--dry-run` and `--fail-on-change`. Reports go to stdout; logs and errors go to stderr. Set `RUST_LOG=hawkeye=debug` to inspect file discovery and Git processing.

Exit code 0 means the selected policy passed. Exit code 1 means `check` found a required change or conflict, an edit command left a conflict, or an enabled failure option matched. Config, I/O, template, and Git errors use exit code 2.

## Integrations

### Docker

The distroless image runs HawkEye in `/workspace`. Pass the host user when formatting a bind mount so that writes retain the expected ownership:

```shell
docker run --rm --user "$(id -u):$(id -g)" --volume "$PWD:/workspace" ghcr.io/fast/hawkeye check
```

### GitHub Actions

Install the released binary and invoke HawkEye directly; no HawkEye-specific action is required:

```yaml
- uses: actions/checkout@v7
- uses: taiki-e/install-action@v2
  with:
    tool: hawkeye
- run: hawkeye check
```

### pre-commit

The default hook installs the matching HawkEye source revision in pre-commit's isolated Python environment:

```yaml
repos:
  - repo: https://github.com/fast/hawkeye
    rev: v7.2.0
    hooks:
      - id: hawkeye-format
```

The hooks pass only the files selected by pre-commit. To scan the complete configured file set whenever a hook runs, set `pass_filenames: false` on that hook in the project's `.pre-commit-config.yaml`; HawkEye then receives no paths and performs a normal full scan. Pre-commit skips a hook when no files match, so also set `always_run: true` when the full scan must run even in that case. The Python hook needs a Rust toolchain when its environment is created for the first time. Use `hawkeye-format-docker` instead when Docker is the preferred runtime.

## Configuration

The following example shows every configuration section. Field names are snake case and unknown fields are rejected.

```toml
[header]
# Choose exactly one source. Built-in keys are case-sensitive.
builtin = "Apache-2.0"
# path = "{{ config_dir }}/HEADER.txt"
# text = "Copyright {{ props.inception_year }} {{ props.copyright_owner }}"

# Every keyword must occur, case-insensitively, before an existing comment can
# be replaced or removed. The default is ["SPDX-"] for the SPDX builtin,
# and ["copyright"] otherwise.
keywords = ["copyright"]

[files]
# Scan the directory containing this config file.
root = "{{ config_dir }}"
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

### Path templates

`files.root` and `header.path` accept MiniJinja templates. A shared config can scan the directory where HawkEye was invoked while keeping its header beside the config. For example, on Unix:

```toml
[files]
root = "{{ cwd }}"

[header]
path = "{{ config_dir }}/HEADER.txt"
```

The two variables are captured when the config is loaded:

| Variable     | Meaning                                                                      |
|--------------|------------------------------------------------------------------------------|
| `cwd`        | The absolute process working directory.                                      |
| `config_dir` | The absolute directory containing the config file, after resolving symlinks. |

Subdirectories can be appended in the same way: `root = "{{ cwd }}/src"`.

Relative paths and relative template results still resolve from `config_dir`, which is also the default scan directory when `root` is omitted. Use `root = "{{ config_dir }}"` to make that choice explicit.

Path templates do not expose environment variables; `props` and `attrs` are available only in header contents. Undefined variables or invalid paths fail config loading.

For configs that also run on Windows, use `join_path` when composing paths, for example `path = "{{ [config_dir, 'HEADER.txt'] | join_path }}"`. This filter joins a list of strings using the platform's native path rules.

### Headers and templates

`header.builtin` accepts `Apache-2.0`, `Apache-2.0-ASF`, `Elastic-2.0`, or `SPDX`. `header.path` loads a UTF-8 template, while `header.text` stores the same template inline. A template file is never selected as a source file.

Templates use MiniJinja with strict undefined values, auto-escaping disabled, and its standard built-in filters, tests, and functions. HawkEye does not enable template includes or external loaders. The template context contains:

| Value                           | Meaning                                                                        |
|---------------------------------|--------------------------------------------------------------------------------|
| `props`                         | The complete user-defined `[props]` table.                                     |
| `attrs.filename`                | The current file name.                                                         |
| `attrs.disk_file_created_year`  | The filesystem creation year, or `null` when unavailable.                      |
| `attrs.disk_file_modified_year` | The filesystem modification year, or `null` when unavailable.                  |
| `attrs.git_file_created_year`   | The year the current exact-path lifetime began, or `null` when unavailable.    |
| `attrs.git_file_modified_year`  | The latest year in the current exact-path history, including worktree changes. |
| `attrs.git_authors`             | Sorted distinct author names from the current exact-path history.              |

HawkEye never substitutes the current year for an unavailable value. Templates that need a fallback must express it explicitly.

#### SPDX headers

The `SPDX` builtin emits an [SPDX license identifier](https://spdx.github.io/spdx-spec/v2.3.1/using-SPDX-short-identifiers-in-source-files/) with optional copyright text. No `header.keywords` configuration is needed:

```toml
[header]
builtin = "SPDX"

[props]
spdx_license_expression = "Apache-2.0 OR MIT"
# Optional; omit this property to emit only the license identifier.
spdx_file_copyright_text = "2020-2026 Acme Developers"
```

For Rust, this produces:

```rust
// SPDX-FileCopyrightText: 2020-2026 Acme Developers
//
// SPDX-License-Identifier: Apache-2.0 OR MIT
```

`spdx_license_expression` is required. Copyright text uses the official [`SPDX-FileCopyrightText` tag](https://spdx.github.io/spdx-spec/v2.3.1/file-tags/). Supply the property values as single-line text; HawkEye renders them as provided without SPDX-specific validation. Include the corresponding license texts in your project as described by the SPDX guidance.

The default keyword `SPDX-` covers both tags so license and copyright values can be updated using the usual header replacement rules. Explicit `header.keywords` overrides this default; an empty list is rejected when parsing the configuration. This template does not provide structured validation or automatic migration of arbitrary existing SPDX declarations; review changes when applying it to existing headers.

### Files, rules, and styles

`files.includes` and `files.excludes` use Git-ignore-style patterns relative to `files.root`; negated patterns are not accepted because inclusion and exclusion are separate lists. An empty `includes` list means all discovered files. `.git` is always excluded. File symlinks are followed, while directory symlinks are not traversed.

Rules match complete filenames or case-insensitive filename suffixes. Extensions omit the leading dot and may contain multiple segments, such as `d.ts`. The first matching user rule wins; built-in rules are lower-priority fallbacks. Duplicate selectors are allowed and logged at debug level when shadowed.

`style_out` is the canonical format written by HawkEye. `styles_in` lists formats that may be recognized and safely replaced or removed. When `styles_in` is empty, it defaults to `[style_out]`; a non-empty list must include `style_out`. Style names are case-sensitive. A custom style may override a built-in style and produces a warning. The bundled mappings are defined in [rules.toml](hawkeye/src/builtin/rules.toml) and [styles.toml](hawkeye/src/builtin/styles.toml).

### Git integration

`git.ignore` defaults to `auto`: it uses the Git index and ignore rules inside a worktree and falls back to filesystem discovery using `.gitignore` files at or below `files.root` outside one. Tracked files remain selected even when they match an ignore rule. Set the mode to `enable` to require a worktree or `disable` to use filesystem discovery without Git ignore rules.

`git.file_attrs` defaults to `disable` because resolving attributes may walk a long history for every selected path. Prefer fixed values in `props` unless the template genuinely needs repository history. `auto` populates attributes when complete history is available; `enable` requires a usable repository and complete history. Selecting paths on the command line also limits Git status and history work to those files.

Git attributes describe the current lifetime of an exact path. Deleting and later recreating a path starts a new history, as does moving a file to a new path; HawkEye does not infer identity from file similarity. At a merge, HawkEye follows a parent whose file entry matches the merge result. A merge resolution that differs from every parent counts as a modification and retains the contributing parent histories.

HawkEye reads repositories in-process with `gix` and does not require a `git` executable at runtime, including in the distroless container image. CI checkouts must still contain complete history when Git file attributes are enabled.

## Library

The library exposes the same engine used by the command-line tool:

```rust
use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::Scope;

let config = Config::load("licenserc.toml")?;
let engine = Engine::new(config)?;
let report = engine.check(Scope::All)?;
# Ok::<(), hawkeye::Error>(())
```

Each operation accepts a `Scope`. `Scope::All` processes the configured file set, while `Scope::Paths(&paths)` processes only the requested files and directories; an empty path slice processes nothing. `Engine::check` never writes files. `Engine::format` and `Engine::remove` return pending `Edits`; call `Edits::apply` to write them or `Edits::into_report` to inspect the result without writing.

`Config::load` resolves path templates once. `Engine::new` uses the resulting paths without rendering them again; paths in a programmatically constructed `Config` are used as supplied.

The default `application` feature builds the command-line tool. Library-only users can omit its command-specific dependencies:

```toml
hawkeye = { version = "7", default-features = false }
```

## Compatibility

HawkEye v7 uses a new snake-case configuration format and does not accept v6 field names. Follow [MIGRATE.md](MIGRATE.md) to migrate a v6 configuration, command line, and integration.

Release summaries are recorded in [CHANGELOG.md](CHANGELOG.md).

The minimum supported Rust version is 1.89.0. It may be raised in a minor release; patch releases preserve the minimum version of their corresponding minor release.

## License

Licensed under the [Apache License, Version 2.0][license-url].
