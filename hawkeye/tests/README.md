# Integration tests

This suite runs the Cargo-built `hawkeye` binary against complete repository fixtures. `test.rs` is the entry point, `test/` groups related behavior, and `test/support.rs` contains the shared temporary-project driver.

Each fixture is copied to a temporary directory before use. Tests may initialize Git, create history, or change the copied files without modifying the checked-in case.

Expected reports and file contents are asserted next to each command. Cargo supplies the exact binary built for the current test invocation:

```shell
cargo x test
```
