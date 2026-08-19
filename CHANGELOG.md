# CHANGELOG

All notable changes to this project will be documented in this file.

## Unreleased

## v7.0.0 (2026-08-19)

### Breaking changes

* Rebuilt HawkEye as one `hawkeye` crate that provides both the Rust library and command-line application; the `hawkeye-fmt` crate and its v6 APIs are not preserved.
* Replaced the v6 camel-case config with a strict snake-case schema organized around `header`, `files`, `props`, `git`, `styles`, and ordered `rules`.
* Removed v6 compatibility switches, default exclude lists, configurable detection regexes, and the repository-specific GitHub Action.
* Changed edit commands to succeed after making changes by default. Use `check` in CI or pass `--fail-on-change` to `format` and `remove`.
* Replaced command-specific JSON files with one report schema written to stdout by `--output-format json`; logs and errors are written to stderr.
* Defined Git creation as the latest addition in the current exact-path lifetime. Delete-and-recreate and rename-to-a-new-path start new histories.

### New features

* Added a public `Config`, `Engine`, `Scope`, report, and pending-edit API.
* Added safe header replacement with explicit conflict reporting, literal line and block styles, and distinct accepted-input and canonical-output styles.
* Added positional file and directory selection for `check`, `format`, and `remove`, including staged-file support in pre-commit hooks.
* Added in-process Git discovery and file attributes through `gix`, including merge-aware history traversal without requiring a `git` executable.
* Added human and JSON reports, stable exit-code categories, debug logging, and deterministic preservation of common file preambles and line endings.
* Added cargo-binstall archives, a distroless multi-architecture container image, and Python and Docker pre-commit hooks.

See [MIGRATE.md](MIGRATE.md) for the v6-to-v7 procedure and semantic differences.

## v6.5.1 (2026-02-14)

### Bug fixes

* Properly resolve relative paths when populating Git attributes for untracked folders.

## v6.5.0 (2026-02-09)

### Notable changes

* Minimal Supported Rust Version (MSRV) is now 1.90.0.

### Bug fixes

* `hawkeye` CLI now uses hawkeye-fmt of exactly the same version to format headers, instead of using the latest version of `hawkeye-fmt` that may not be compatible with the current version of `hawkeye`.

### Improvements

* Replace `anyhow` with `exn` for more informative error messages.

## v6.4.2 (2026-02-07)

### Bug fixes

* Set Git attributes for untracked folders as if it were committed now.

## v6.4.1 (2026-01-13)

### Improvements

* Use `TextLayout` for logging output to improve formatting and readability.

## v6.4.0 (2026-01-12)

### Notable changes

* `attrs.disk_file_created_year`, `attrs.git_file_created_year`, and `attrs.git_file_modified_year` are now integers instead of strings. Most use cases should not be affected.
* `attrs.git_file_created_year` is now set even if the file is not tracked by Git. In this case, it will be set to the current year (as if it were committed now).
* `attrs.git_file_modified_year` is now overwritten if the file is modified but not committed by Git. In this case, it will be set to the current year (as if it were committed now).
* `attrs.disk_file_created_year` is then soft-deprecated. It can still be set, but it is recommended to use `attrs.git_file_created_year` and `attrs.git_file_modified_year` directly instead.

The semantic changes above are breaking, but they should not affect most users and should always be what you want.

* `additionalHeaders` and `headerPath` now search from the following paths in order:
  1. The directory of the configuration file, a.k.a., config_dir.
  2. The configured baseDir.
  3. The current working directory.

### Improvements

* If `--config` is not specified, HawkEye will now search for `.licenserc.toml` in addition to `licenserc.toml`.

## v6.3.0 (2025-10-09)

### New features

* Add distribution against musl libc ([#196](https://github.com/fast/hawkeye/pull/196)).

## v6.2.0 (2025-08-25)

### New features

* Supports format Vue files: pattern = "vue" and headerType = "XML_STYLE".
* Supports format Containerfile files: pattern = "Containerfile" and headerType = "SCRIPT_STYLE".
* Add a shared flag to store lists of files to change ([#194](https://github.com/fast/hawkeye/pull/194)).

## v6.1.1 (2025-06-11)

### New features

* Supports format CommonJS files: pattern = "cjs" and headerType = "SLASHSTAR_STYLE".
* Supports format Verilog files: pattern = "v" and headerType = "SLASHSTAR_STYLE".
* Supports format SystemVerilog files: pattern = "sv" and headerType = "SLASHSTAR_STYLE".

## v6.1.0 (2025-06-06)

### New features

* `attrs.disk_file_created_year` can be used to replace nonexisting Git attrs like `{{attrs.git_file_created_year if attrs.git_file_created_year else attrs.disk_file_created_year }}`

## v6.0.0 (2025-01-28)

### Breaking changes

Now, HawkEye uses MiniJinja as the template engine.

All the `properties` configured will be passed to the template engine as the `props` value, and thus:

* Previous `${property}` should be replaced with `{{ props["property"] }}`.
* Previous built-in variables `hawkeye.core.filename` is now `attrs.filename`.
* Previous built-in variables `hawkeye.git.fileCreatedYear` is now `attrs.git_file_created_year`.
* Previous built-in variables `hawkeye.git.fileModifiedYear` is now `attrs.git_file_modified_year`.

New properties:

* `attrs.git_authors` is a collection of authors of the file. You can join them with `, ` to get a string by `{{ attrs.git_authors | join(", ") }}`.

### Notable changes

Now, HawkEye would detect a leading BOM (Byte Order Mark) and remove it if it exists (#166). I tend to treat this as a bug fix, but it may affect the output of the header.
