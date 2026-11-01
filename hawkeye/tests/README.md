# Behavior tests

Tests exercise the public Rust API and the Cargo-built `hawkeye` command against temporary projects. Assertions cover observable reports, errors, exit codes, and file contents rather than private helpers or implementation branches.

[library.rs](library.rs) covers configuration validation, scope selection, and the prepare/inspect/discard/apply lifecycle, including stopping at the first error without rolling back earlier writes and preserving hard links when editing a single selected path. It runs both with default features disabled and with all features enabled.

[test.rs](test.rs) runs the CLI scenarios with the shared temporary-project driver in [support.rs](test/support.rs):

| Scenarios                                | Behavior guarded                                                                                                           |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| [command.rs](test/command.rs)            | Reports, exit policies, dry runs, config lookup, requested paths, consistent filtering, and report errors before writes.   |
| [config.rs](test/config.rs)              | Rule precedence, custom styles, recognition keywords, and actionable configuration errors.                                 |
| [format.rs](test/format.rs)              | Header lifecycle, SPDX, style conversion, preservation of source text and preambles, conflicts, and template exclusion.    |
| [frontmatter.rs](test/frontmatter.rs)    | Markdown metadata boundaries, formatting and removal, line endings, and incomplete or non-YAML prefixes.                   |
| [paths.rs](test/paths.rs)                | Path-template contexts, relative roots, platform-specific paths, and diagnostics without environment-variable exposure.    |
| [git.rs](test/git.rs)                    | Discovery and ignores, file lifetimes, branch and merge history, shallow repositories, and operation without a Git binary. |

Checked-in [cases](cases) are copied to temporary directories before use. Tests may initialize Git, create history, or change copied files without modifying the fixture. Platform-specific scenarios run on the corresponding CI runners. Expected reports and file contents are asserted next to each operation.

Run both suites through the repository workflow:

```shell
cargo x test
```

Add scenarios for new user-visible behavior or a concrete regression. Prefer extending an existing lifecycle or boundary case to adding a test for each internal method or builtin mapping.
