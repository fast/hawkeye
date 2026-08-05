# HawkEye

HawkEye checks, rewrites, and removes source-file license headers. The `hawkeye` Cargo package contains both a reusable Rust library and a thin command-line executable.

> [!WARNING]
> Version 7 is an intentional rewrite with a new configuration and API. Compatibility with v6 is not a goal, and `7.0.0-alpha.1` should be treated as an unstable preview until it is published.

## Try the rewrite

Install the current checkout:

```console
cargo install --path .
```

Create `hawkeye.toml` in the repository root:

```toml
[header]
text = """Copyright {{ year }} Example Developers

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License."""
identifiers = ["Copyright", "Apache License, Version 2.0"]

[variables]
year = 2026

[files]
use_gitignore = true
exclude = ["**/generated/**"]
```

Common languages use built-in rules, so the minimal configuration needs no language mapping. Run:

```console
hawkeye check
hawkeye format --dry-run --diff
hawkeye format
```

`check` never writes. `format` adds missing headers and replaces recognized stale or non-preferred headers. `remove` only deletes a header whose exact source range was structurally recognized.

## Configuration

HawkEye rejects unknown fields. All names use `snake_case`, all relative paths resolve against the directory containing `hawkeye.toml`, and ordered rules use first-match precedence.

### Header and variables

`[header]` must set exactly one of `text` or `path`, plus one or more `identifiers`. A path points to unstyled header text and is never scanned as a source file. MiniJinja renders the header once per run from `[variables]`; missing variables are errors.

Identifiers provide explicit evidence for recognizing stale text inside a structurally parsed comment. Use specific phrases that distinguish the configured license from ordinary source comments.

```toml
[header]
path = "headers/Apache-2.0-ASF.txt"
identifiers = ["Apache Software Foundation", "Apache License, Version 2.0"]

[variables]
year = 2026
project = "example"
```

### File selection

Discovery includes hidden and untracked files, skips `.git`, never follows symlinks, and honors Git ignore files by default. `include` narrows the candidate set and `exclude` removes paths from it.

```toml
[files]
use_gitignore = true
include = ["src/**", "tests/**"]
exclude = ["**/fixtures/**", "**/generated/**"]
```

Paths that pass discovery but match no rule are reported as `unsupported`. This is informational and does not fail `check`.

### Rules and styles

Rules are evaluated in declaration order before built-in defaults. `write_style` is the canonical output style. `read_styles` lists additional styles that may be safely recognized and migrated; the write style is always readable automatically.

```toml
[[rules]]
patterns = ["*.rs", "**/*.rs"]
write_style = "slash"
read_styles = ["slash_star"]
```

Built-in styles are `slash` (`//`), `hash` (`#`), `dash` (`--`), `slash_star` (`/* ... */`), and `xml` (`<!-- ... -->`). The built-in language rules cover common Rust, Go, C/C++, JVM, JavaScript/TypeScript, Python, Ruby, shell, TOML, YAML, SQL, and XML-family files, plus conventional build filenames such as `Dockerfile`, `Makefile`, and `CMakeLists.txt`.

Set `use_default_rules = false` to require only explicit rules. Custom line and block styles are supported:

```toml
use_default_rules = false

[styles.semicolon]
kind = "line"
prefix = ";; "

[styles.template_block]
kind = "block"
start = "{{!"
prefix = "  "
end = "}}"

[[rules]]
patterns = ["*.lisp", "**/*.lisp"]
write_style = "semicolon"
```

## Commands and exit codes

```text
hawkeye [--config PATH] [--output-format human|json] check [--diff]
hawkeye [--config PATH] [--output-format human|json] format [--dry-run] [--diff]
hawkeye [--config PATH] [--output-format human|json] remove [--dry-run] [--diff]
```

`--diff` writes unified diffs to stdout and cannot be combined with JSON output. Normal `format` and `remove` runs apply every safe edit, leave conflicts byte-for-byte unchanged, and return a finding exit code if conflicts remain.

- `0`: the command completed and its policy was satisfied.
- `1`: `check` found a violation, a dry run has pending changes, or a conflict remains.
- `2`: invocation, configuration, template, discovery, or I/O failure.

## Library

The pure analyzer can be embedded without enabling filesystem mutation or CLI policy:

```rust
use hawkeye::{Analyzer, Config, Mode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = r#"
[header]
text = "Copyright 2026 Example Developers"
identifiers = ["Copyright"]
"#;
    let config = Config::from_toml(raw)?.resolve(".")?;
    let analyzer = Analyzer::new(&config, "Copyright 2026 Example Developers")?;
    let source = "fn main() {}\n";
    let plan = analyzer.plan("src/main.rs", source, Mode::Format)?;
    let rewritten = plan.apply(source)?;
    assert!(rewritten.starts_with("// Copyright 2026 Example Developers"));
    Ok(())
}
```

Use `Engine::load` when repository discovery, safe atomic replacement, and a deterministic `Report` are desired. The detailed state machine and safety contracts are recorded in [`docs/v7-design.md`](docs/v7-design.md).

## Development

```console
cargo fmt --all --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test --all-features
cargo test --no-default-features
cargo run -- check
```

The v6 source remains preserved separately for reference; v7 is not implemented as an incremental migration of that codebase.

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).
