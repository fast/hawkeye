# CHANGELOG

All notable changes to this project will be documented in this file.

## Unreleased

### Breaking changes

The license-header configuration model has changed. The style-keyed `[mapping.STYLE_NAME]` blocks and the `useDefaultMapping` option are removed; configure file handling with a per-language `[[headers]]` rule list instead:

* Replace each `[mapping.STYLE] { extensions = ["x"], filenames = ["y"] }` with:
  ```toml
  [[headers]]
  extensions = ["x"]
  filenames = ["y"]
  styles = ["STYLE"]
  ```
* Default language-to-style rules stay active; declare `[[headers]]` only to override or extend them. `useDefaultMapping` is renamed to `useDefaultHeaders`.

`hawkeye format` no longer stacks a duplicate header when a file already has one written in a comment style different from the configured one. An existing header is detected by a style-agnostic scan for the configured `keywords`. Each `[[headers]]` rule chooses what happens to such a file via `existingStrategy`:

* `"replace"` (default): remove the existing header and write the preferred style. When a rule lists more than one style, a header in any listed style is removed, so it migrates to the preferred (first) style instead of being duplicated. If the file looks licensed but the command cannot normalize it to the preferred style (the header is in an unlisted comment style, or only a stray keyword is present), the file is left unchanged and the run fails (see the breaking note below).
* `"skip"`: leave the file untouched and report it; the run still succeeds (exit 0).
* `"error"`: fail the run.

Detection is a keyword scan, so it has a blind spot in the other direction too: a real notice that contains none of the configured `keywords` (an MIT/BSD notice with no `copyright` line, or the keyword-less tail of a header split by an inserted blank line) is not detected and is left below the rewritten header. Tune `keywords` if this or the stray-keyword case bites.

The `keywords` scan is now case-insensitive as documented: configured keywords are lowercased before matching, so a custom `keywords = ["Copyright"]` now matches a `copyright` notice. In 6.x a capitalized custom keyword silently never matched. The default (`copyright`) is unaffected.

`existingStrategy` governs `hawkeye format` only; `hawkeye check` and `hawkeye remove` ignore it.

Under `existingStrategy = "replace"`, a license-looking header that cannot be normalized to the preferred style now fails the run with a non-zero exit code. In 6.x the file was left unchanged and the run still exited 0, silently passing an un-normalized file through CI. The file is still left untouched; only the exit code changed. `"skip"` is unaffected and still exits 0.

`hawkeye check` now reports files that carry a non-matching existing header (a stale header in a recognized style, or a notice in a style the rule does not list) as a distinct `foreign` category, separate from files missing a header entirely. Because detection includes the same keyword scan, a file that has no header but contains a stray keyword near the top is reported as `foreign` rather than `missing`.

The `--output` JSON uses one grammar across `check`, `format`, and `remove`: every result field that lists files is a list of file paths categorized by the field name (`unknown`, `missing`, `foreign`, `skipped`, `conflict`, `removed`). `format`'s `updated` is the same list shape but its entries are suffixed `path=added` or `path=replaced`. `remove`'s `removed` entries are now bare file paths (previously `path=removed`), so scripts that split on `=` must be updated. `format` and `remove` additionally carry a boolean `dry_run` field (not a path list).

The `foreign` field means different things per command. `check.foreign` is any non-matching existing header (stale, listed-foreign-style, or unlisted), found by the structural parse or the keyword scan. `format.foreign` and `remove.foreign` are narrower: only what the command could not handle - an unlisted comment style, or a stray keyword (`format` could not normalize to the preferred style; `remove` could not locate a removable block). A stale or listed-foreign-style header is migrated/removed instead, so it never appears in their `foreign` field.

`hawkeye format` and `hawkeye remove` now write files atomically (write a sibling temp file, fsync, then rename over the target) instead of editing in place. Consequences:

* If a target file is a symlink, it is replaced by a regular file (the 6.x in-place write followed the link). The replacement carries the link target's permission bits.
* Writing now hinges on the parent directory rather than the file. Creating the temp file and renaming it over the target need write permission on the directory, not on the file, so a read-only source file can now be rewritten (the rename replaces the directory entry), while a writable file in a read-only directory now fails the run instead of being rewritten.

File permissions are preserved on a best-effort basis; ownership and ACLs are not.

`hawkeye` now processes a file matched by a `.gitignore` rule when that file is tracked in the Git index (for example, force-added with `git add -f`); 6.x skipped every gitignore-matched file (the #209 fix - a force-added file should still get a header). On upgrade such a file is checked for the first time and can newly appear in `missing` or `foreign`, failing a previously-green run. List the newly-processed set with `git ls-files | git check-ignore --stdin --no-index` and add any you do not want checked to `excludes`.

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
