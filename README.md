# HawkEye

[![Crates.io][crates-badge]][crates-url]
[![Documentation][docs-badge]][docs-url]
[![MSRV 1.85][msrv-badge]](https://www.whatrustisit.com)
[![Apache 2.0 licensed][license-badge]][license-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/hawkeye.svg
[crates-url]: https://crates.io/crates/hawkeye
[docs-badge]: https://img.shields.io/docsrs/hawkeye
[docs-url]: https://docs.rs/hawkeye
[msrv-badge]: https://img.shields.io/badge/MSRV-1.85-green?logo=rust
[license-badge]: https://img.shields.io/crates/l/hawkeye
[license-url]: https://www.apache.org/licenses/LICENSE-2.0
[actions-badge]: https://github.com/korandoru/hawkeye/actions/workflows/ci.yml/badge.svg
[actions-url]: https://github.com/korandoru/hawkeye/actions/workflows/ci.yml

HawkEye is a license header checker and formatter.

HawkEye v7 is being rebuilt from first principles. The current implementation defines the versioned configuration contract and its validation boundary; file discovery, header analysis, editing, and the CLI will be reintroduced incrementally after their behavior and public contracts have been reviewed against v6.

## Development

Repository workflows are exposed through `cargo x`:

```shell
cargo x build
cargo x test
cargo x lint
```

The root `hawkeye` package produces both the library and the command-line binary. The `xtask` package is an unpublished development tool.

## Minimum Rust version policy

The minimum supported Rust version is 1.85.0.

The minimum supported Rust version may be increased in a minor release. Patch releases will preserve the minimum supported Rust version of their corresponding minor release.

## License

This project is licensed under [Apache License, Version 2.0][license-url].
