# Integration tests

This unpublished workspace package exercises the released `hawkeye` binary against complete repository trees rather than isolated internal functions.

Each directory under `cases` is copied to a temporary directory before a test runs. Tests may then initialize Git, populate its index, configure ignore sources, create branches and dated commits, or add dirty and untracked files. The checked-in corpus is never modified in place.

The snapshots under `tests/snapshots` record observable behavior at the repository boundary: the initial tree, the first `check` report, the `format` report, the resulting byte-preserving tree, the clean `check`, and an idempotent second `format`. Tree snapshots make BOM, LF, CRLF, exclusions, conflicts, and untouched ignored files visible.

Run the suite through the repository workflow:

```shell
cargo x test
```

When behavior changes intentionally, run the integration test with `INSTA_UPDATE=new`, inspect the generated `.snap.new` files with `cargo insta review`, and commit only accepted snapshots.
