# Integration tests

This package runs the Cargo-built `hawkeye` binary against complete repository fixtures. `tests/test.rs` is the suite entry point, its neighboring modules group related behavior, and `src/lib.rs` contains the shared temporary-project driver.

Each fixture is copied to a temporary directory before use. Tests may initialize Git, create history, or change the copied files without modifying the checked-in case.

Expected reports and file contents are asserted next to each command. Run the suite through the repository workflow so the `hawkeye` binary is available to the test driver:

```shell
cargo x test
```
