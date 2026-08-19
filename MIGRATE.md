# Migrating from HawkEye v6 to v7

HawkEye v7 is a rewrite. It does not accept v6 configuration aliases, and several behaviors changed in addition to the move from camel case to snake case. This guide targets v6.5.x and v7.0.0-alpha.1 and is written as an executable migration checklist for either a person or a code agent.

## Migration rules

- Work on a clean branch and preserve the existing v6 config until the v7 result has been verified.
- Do not add compatibility aliases to the v7 config. Replace every v6 field and invocation at its source.
- Preserve the rendered license text, selected files, and failure policy unless this guide identifies an intentional v7 semantic change.
- Treat exit code 2 as a migration or runtime error. Exit code 1 from `check` normally means the config loaded successfully but one or more files need attention.
- Review a dry-run report before allowing `format` to write files. Do not silently accept `conflict` outcomes.
- The canonical prerelease version is `7.0.0-alpha.1`, including the dot before `1`.

## 1. Inventory the v6 setup

Before editing, find every HawkEye input and integration:

```shell
find . -name 'licenserc.toml' -o -name '.licenserc.toml'
rg -n 'hawkeye|hawkeye-fmt|headerPath|inlineHeader|additionalHeaders|useDefault' .
```

Record the following before conversion:

- The selected config file and its `baseDir`.
- Whether the header is inline, bundled, or loaded from a file.
- Every additional header-style file referenced by `additionalHeaders`.
- Custom entries under `[mapping.*]` and whether `useDefaultMapping` is disabled.
- Negated exclude patterns and directories that were covered only by v6 default excludes.
- CLI flags whose exit-code behavior is relied on by CI.
- GitHub Actions, Docker, pre-commit, Cargo, or library integrations.

## 2. Replace the config structure

The following v6 config:

```toml
baseDir = "."
headerPath = "Apache-2.0.txt"
strictCheck = true
useDefaultExcludes = true
useDefaultMapping = true
includes = ["**/*.rs", "**/*.toml"]
excludes = ["generated/**"]
keywords = ["copyright"]

[properties]
inceptionYear = 2022
copyrightOwner = "Acme Developers"

[git]
ignore = "auto"
attrs = "disable"
```

becomes:

```toml
[header]
builtin = "Apache-2.0"
keywords = ["copyright"]

[files]
root = "."
includes = ["**/*.rs", "**/*.toml"]
excludes = ["generated/**"]

[props]
inception_year = 2022
copyright_owner = "Acme Developers"

[git]
ignore = "auto"
file_attrs = "disable"
```

Use this field mapping for the rest of the config:

| v6 | v7 | Migration action |
| --- | --- | --- |
| `baseDir` | `files.root` | Move the value under `[files]`. |
| `inlineHeader` | `header.text` | Move the template under `[header]`. |
| `headerPath` | `header.builtin` or `header.path` | Use a built-in key for a bundled header; otherwise use a path. |
| `keywords` | `header.keywords` | Move the list under `[header]`. |
| `includes` | `files.includes` | Move the list under `[files]`. |
| `excludes` | `files.excludes` | Move the list and rewrite any negation. |
| `properties` | `props` | Move values and update every template reference. |
| `git.ignore` | `git.ignore` | No field rename. |
| `git.attrs` | `git.file_attrs` | Rename the field. |
| `[mapping.STYLE]` | `[[rules]]` | Convert mappings to ordered rule entries. |
| `additionalHeaders` | `[styles.NAME]` | Inline each referenced style in the main config. |
| `strictCheck` | none | Remove it; v7 uses safe literal style recognition. |
| `useDefaultExcludes` | none | Remove it and make required exclusions explicit. |
| `useDefaultMapping` | none | Remove it; built-in rules are always low-priority fallbacks. |

v6 evaluated a relative `baseDir` from the process working directory. v7 resolves a relative `files.root` from the directory containing the selected config file, so rewrite the path when those directories differ. Relative `header.path` values are also resolved only from the config directory; v6 additionally tried `baseDir` and the process working directory for resources.

### Header source

v7 requires exactly one header source:

- `header.builtin = "Apache-2.0"`
- `header.builtin = "Apache-2.0-ASF"`
- `header.builtin = "Elastic-2.0"`
- `header.path = "path/to/header.txt"`
- `header.text = "Copyright ..."`

When v6 sets both `inlineHeader` and `headerPath`, it silently prefers the inline value. Preserve that result by keeping only `header.text` in v7.

The bundled v6 filenames lose their `.txt` suffix when used as v7 built-in keys. A custom file named `Apache-2.0.txt` must still use `header.path` if its content differs from the bundled template.

### Properties and templates

v6 converts every property to a string. v7 exposes the original TOML value under `props`, so integers remain integers, booleans remain booleans, and nested arrays or tables are allowed. Recheck comparisons, filters, and fallback expressions that assumed strings.

The v7 bundled Apache and Elastic templates use snake-case property names. Convert:

```jinja
{{props["inceptionYear"]}} {{props["copyrightOwner"]}}
```

to:

```jinja
{{ props.inception_year }} {{ props.copyright_owner }}
```

Custom property keys are not restricted, but migrating them to snake case makes the config consistent. Rename each key and every reference together.

v7 configures MiniJinja with strict undefined values. Missing properties now fail instead of rendering an empty value. File attributes that cannot be determined are `null`; express any fallback in the template rather than assuming the current year.

The available attributes are:

- `attrs.filename`
- `attrs.disk_file_created_year`
- `attrs.disk_file_modified_year`
- `attrs.git_file_created_year`
- `attrs.git_file_modified_year`
- `attrs.git_authors`

### File selection

An empty `files.includes` list selects every discovered file. Both include and exclude patterns are relative to `files.root` and use Git-ignore-style matching.

v7 does not accept `!` negation in either list. Rewrite a v6 exclusion such as `build/**` plus `!build/keep.rs` so the broad exclusion no longer covers the file that must remain selected. Do not copy the negated pattern verbatim.

v6 maintained a large default blacklist for build output, dependency directories, editor state, and metadata files. v7 removes that blacklist. Git ignore rules handle these paths in a normal repository; add project-specific `files.excludes` when generated or vendored files are not ignored, especially when `git.ignore = "disable"` or the scan runs outside a repository.

The header template selected by `header.path` is automatically excluded. `.git` is always excluded. v7 follows file symlinks but does not recurse through directory symlinks.

### Rules and styles

v6 mappings are keyed by style and stored without a useful declaration order. v7 rules are an ordered list. User rules run before built-in rules, and the first matching user rule wins.

Convert this v6 mapping:

```toml
[mapping.DOUBLESLASH_STYLE]
extensions = ["foo"]
filenames = ["Foo.config"]
```

to:

```toml
[[rules]]
extensions = ["foo"]
filenames = ["Foo.config"]
style_out = "doubleslash"
```

`style_out` is the canonical style written by `format`. An empty `styles_in` accepts only `style_out`. When existing files legitimately use more than one style, list the complete accepted set and include `style_out`:

```toml
styles_in = ["doubleslash", "slashstar"]
```

Do not recreate a v6 mapping that is already covered by a v7 built-in rule unless the project needs to override its output or accepted styles. The current built-in selectors are documented in [`rules.toml`](hawkeye/src/builtin/rules.toml).

v6 style names are case-insensitive; v7 names are case-sensitive. These common built-in names map as follows:

| v6 | v7 | v6 | v7 |
| --- | --- | --- | --- |
| `DOUBLESLASH_STYLE` | `doubleslash` | `SLASHSTAR_STYLE` | `slashstar` |
| `TRIPLESLASH_STYLE` | `tripleslash` | `JAVADOC_STYLE` | `javadoc` |
| `SCRIPT_STYLE` | `script` | `DOUBLEDASHES_STYLE` | `doubledashes` |
| `XML_STYLE` | `xml` | `XML_PER_LINE` | `xml_per_line` |
| `PERCENT_STYLE` | `percent` | `PERCENT3_STYLE` | `percent3` |
| `SEMICOLON_STYLE` | `semicolon` | `APOSTROPHE_STYLE` | `apostrophe` |
| `EXCLAMATION_STYLE` | `exclamation` | `EXCLAMATION3_STYLE` | `exclamation3` |
| `DOUBLETILDE_STYLE` | `doubletilde` | `BATCH` | `batch` |
| `HAML_STYLE` | `haml` | `BRACESSTAR_STYLE` | `bracesstar` |
| `SHARPSTAR_STYLE` | `sharpstar` | `MUSTACHE_STYLE` | `mustache` |
| `MVEL_STYLE` | `mvel` | `FTL` | `ftl` |
| `FTL_ALT` | `ftl_alt` | `DYNASCRIPT_STYLE` | `dynascript` |
| `DYNASCRIPT3_STYLE` | `dynascript3` | `ASP` | `asp` |
| `PHP` | `slashstar` | `LUA` | `lua` |
| `ASCIIDOC_STYLE` | `asciidoc` | `LINE_BLOCK_STYLE` | `line_block` |

For `SCALA_STYLE`, `JAVAPKG_STYLE`, `UNKNOWN`, or any project-defined style, create a custom v7 style instead of guessing a built-in replacement. Even mapped built-ins may produce slightly different canonical whitespace, so inspect the dry-run diff.

Move every style from files named by `additionalHeaders` into the main config. A v6 line style:

```toml
[MY_STYLE]
multipleLines = false
beforeEachLine = "// "
afterEachLine = ""
padLines = false
```

becomes:

```toml
[styles.my_style]
kind = "line"
prefix = "// "
suffix = ""
pad_lines = false
```

For a v6 style with `multipleLines = true`, use `kind = "block"`, map `firstLine` to `start`, `beforeEachLine` to `prefix`, `afterEachLine` to `suffix`, and `endLine` to `end`. Remove newline sentinels from `start` and `end`; v7 controls line endings itself.

v7 derives recognition from the literal line or block delimiters. It has no configurable equivalents for `firstLineDetectionPattern`, `lastLineDetectionPattern`, `skipLinePattern`, or `allowBlankLines`. It natively preserves a UTF-8 BOM, shebangs, XML and PHP declarations, HTML doctypes, YAML directives, and common Python or Ruby magic comments. If a project relies on another `skipLinePattern`, stop and report that case instead of silently dropping it.

`strictCheck = false` also has no direct replacement. v7 does not use whitespace-folded or similarity-based matching. A loosely matched v6 header may become `replace` when it is safely recognized or `conflict` when it is not safe to edit automatically.

### Git attributes

Rename `git.attrs` to `git.file_attrs`. Both Git fields accept `"disable"`, `"auto"`, or `"enable"`.

v7 defines Git creation as the latest addition in the current exact-path lifetime. Delete-and-recreate and rename-to-a-new-path both start a new lifetime. Merge history follows the parent state that produced the current file. This intentionally differs from treating every historical occurrence of the path as one continuous file.

`git.file_attrs` remains disabled by default because a long-lived file can require substantial history traversal. `enable` requires a complete, non-shallow repository; `auto` falls back when Git attributes are unavailable. v7 reads Git data in-process with `gix` and does not require a `git` executable at runtime.

## 3. Update command lines

| v6 | v7 |
| --- | --- |
| `--fail-if-unknown` | `--fail-on-unknown` |
| `--output report.json` | `--output-format json > report.json` |
| `check --fail-if-missing=true` | `check` already fails when a change is required |
| `check --fail-if-missing=false` | No equivalent; consume the JSON report if a non-failing audit is required |
| `format --fail-if-updated=true` | `format --fail-on-change` |
| `format --fail-if-updated=false` | `format` |
| `remove --fail-if-updated=true` | `remove --fail-on-change` |
| `remove --fail-if-updated=false` | `remove` |

The v7 `format` and `remove` commands succeed after making changes unless `--fail-on-change` is present. CI should normally use `hawkeye check`; use `--fail-on-change` only when an editing command must preserve the old failure policy.

v7 `--dry-run` performs no writes. v6 created sibling `.formatted` or `.removed` files; remove any workflow that reads those files and consume the human or JSON report instead.

All reports now go to stdout, while logs and errors go to stderr. JSON has one schema for every command:

```json
{
  "files": [
    {
      "path": "src/lib.rs",
      "outcome": "replace"
    }
  ]
}
```

The possible outcomes are `clean`, `add`, `replace`, `remove`, `conflict`, and `unsupported`.

v7 also accepts files and directories after the subcommand. With no paths it scans the configured file set:

```shell
hawkeye check
hawkeye check src/lib.rs src/bin
```

## 4. Update integrations

### GitHub Actions

The v6 repository action is removed. Install and invoke the released CLI explicitly:

```yaml
- uses: actions/checkout@v7
- uses: taiki-e/install-action@v2
  with:
    tool: hawkeye@7.0.0-alpha.1
- run: hawkeye check
```

### Docker

Use the transferred image namespace and mount the repository at `/workspace`:

```shell
docker run --rm --user "$(id -u):$(id -g)" --volume "$PWD:/workspace" ghcr.io/fast/hawkeye:v7.0.0-alpha.1 check
```

### pre-commit

Update the repository and revision:

```yaml
repos:
  - repo: https://github.com/fast/hawkeye
    rev: v7.0.0-alpha.1
    hooks:
      - id: hawkeye-format
```

v7 hooks pass pre-commit's selected files to HawkEye. To retain a deliberate full-repository scan, set both `pass_filenames: false` and `always_run: true` in the consuming repository.

### Rust library

The separate `hawkeye-fmt` crate is replaced by the `hawkeye` library. A minimal dependency is:

```toml
hawkeye = { version = "7.0.0-alpha.1", default-features = false }
```

The v6 callback and document APIs are not preserved. Build a `Config`, create an `Engine`, and select an explicit `Scope`:

```rust
use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::Scope;

let config = Config::load("licenserc.toml")?;
let engine = Engine::new(config)?;
let report = engine.check(Scope::All)?;
# Ok::<(), hawkeye::Error>(())
```

`Engine::format` and `Engine::remove` return pending edits. Call `apply` to write them or `into_report` to inspect them without writing.

## 5. Verify the migration

Install the exact prerelease and confirm the selected binary:

```shell
cargo install hawkeye --version 7.0.0-alpha.1 --locked
hawkeye --version
```

Then verify in this order:

1. Run `hawkeye check --output-format json > hawkeye-report.json`. Exit code 1 is expected when files need changes; exit code 2 means the config or runtime still needs repair.
2. Inspect every `conflict` and `unsupported` entry. Fix the rule, style, template, or file selection rather than forcing an edit.
3. Run `hawkeye format --dry-run --output-format json > hawkeye-format.json` and review all `add` and `replace` outcomes.
4. Ensure the Git worktree is clean, run `hawkeye format`, and review `git diff` for header text, whitespace, preambles, and line endings.
5. Run `hawkeye check` again. It must exit 0.
6. Run the project's normal tests and its GitHub Actions, Docker, or pre-commit integration as applicable.
7. Remove obsolete v6 style files and integration code only after the v7 check passes.

For a complete annotated v7 config, see the [Configuration](README.md#configuration) section in the README.
