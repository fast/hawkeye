# CHANGELOG

All notable changes to this project will be documented in this file.

## Unreleased

### Notable changes

* `check`, `format`, and `remove` accept the files and directories to process as arguments, or with `--files-from <FILE>` (`-` to read them from stdin). This skips walking the whole `baseDir`, as well as the parts of the Git history that are irrelevant to those files, which is significantly faster on large repositories. Files passed explicitly are processed even if Git ignores them, and a file that was deleted and added again reports the most recent addition as `attrs.git_file_created_year`.
* Resolving `git.attrs` is much faster: the history is traversed with an object cache, each commit is diffed against its own parent instead of against the previously visited commit, and the traversal is spread over the available cores. On a repository of 23.5k commits and 5k files, that took the traversal from 57s to 1.2s.

### Bug fixes

* `attrs.git_file_created_year` and `attrs.git_file_modified_year` are no longer misattributed in repositories with merge commits. The history traversal used to diff commits that are not related to each other, which reported the date of an unrelated commit for the files in between; on a repository of 23.5k commits, 2% of the files rendered a wrong year. Merge commits are now skipped, as `git log` does by default, so changes are attributed to the commit that made them.

## [6.5.1] 2026-02-14

### Bug fixes

* Properly resolve relative paths when populating Git attributes for untracked folders.

## [6.5.0] 2026-02-09

### Notable changes

* Minimal Supported Rust Version (MSRV) is now 1.90.0.

### Bug fixes

* `hawkeye` CLI now uses hawkeye-fmt of exactly the same version to format headers, instead of using the latest version of `hawkeye-fmt` that may not be compatible with the current version of `hawkeye`.

### Improvements

* Replace `anyhow` with `exn` for more informative error messages.

## [6.4.2] 2026-02-07

## Bug fixes

* Set Git attributes for untracked folders as if it were committed now.

## [6.4.1] 2026-01-13

## Improvements

* Use `TextLayout` for logging output to improve formatting and readability.

## [6.4.0] 2026-01-12

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

## Improvements

* If `--config` is not specified, HawkEye will now search for `.licenserc.toml` in addition to `licenserc.toml`.

## [6.3.0] 2025-10-09

### New features

* Add distribution against musl libc ([#196](https://github.com/korandoru/hawkeye/pull/196)).

## [6.2.0] 2025-08-25

### New features

* Supports format Vue files: pattern = "vue" and headerType = "XML_STYLE".
* Supports format Containerfile files: pattern = "Containerfile" and headerType = "SCRIPT_STYLE".
* Add a shared flag to store lists of files to change ([#194](https://github.com/korandoru/hawkeye/pull/194)).

## [6.1.1] 2025-06-11

### New features

* Supports format CommonJS files: pattern = "cjs" and headerType = "SLASHSTAR_STYLE".
* Supports format Verilog files: pattern = "v" and headerType = "SLASHSTAR_STYLE".
* Supports format SystemVerilog files: pattern = "sv" and headerType = "SLASHSTAR_STYLE".

## [6.1.0] 2025-06-06

### New features

* `attrs.disk_file_created_year` can be used to replace nonexisting Git attrs like `{{attrs.git_file_created_year if attrs.git_file_created_year else attrs.disk_file_created_year }}`

## [6.0.0] 2025-01-28

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
