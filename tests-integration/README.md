# Integration test cases

The test harness lives at `hawkeye/tests/test.rs`, where Cargo exposes the package's real `hawkeye` binary without another build step or binary-path discovery. This directory contains only repository trees that are useful to inspect as data.

Each case is copied into a fresh temporary worktree before a test runs. Tests may then initialize Git, populate its index, configure ignore sources, create dated history, or add dirty and untracked files. The checked-in cases are never modified.

Expected reports and changed file contents are asserted next to the action in the corresponding test module. There is no snapshot update workflow: a behavior change must make the new expectation explicit in code.

Run the complete suite through the repository workflow:

```shell
cargo x test
```
