# Release HawkEye

Releases start with a preparation PR. After it is merged, run `cargo release` locally to publish the crate and push a signed version tag. The tag triggers GitHub Actions to publish binary distributions and container images.

| Stage       | Entry point                                                        | Output                                                    |
| ----------- | ------------------------------------------------------------------ | --------------------------------------------------------- |
| Preparation | Pull request                                                       | Versions, changelog, and README ready for release          |
| Publication | `cargo release --package hawkeye --execute`                         | crates.io package and signed `vX.Y.Z` tag pushed to origin  |
| Binaries    | Tag push triggers [Release](.github/workflows/release.yml)           | GitHub Release with archives, checksums, and an installer  |
| Containers  | Tag push triggers [Docker](.github/workflows/docker.yml)             | `ghcr.io/fast/hawkeye:vX.Y.Z` for Linux amd64 and arm64      |

Pushing a tag alone does not publish the crate to crates.io. Pull requests only validate the cargo-dist release plan; they do not build or publish the release archives.

## Prerequisites

* Install [cargo-release](https://github.com/crate-ci/cargo-release) (`cargo install cargo-release --locked`; this process was verified with 1.1.2).
* Install the cargo-dist version specified by `cargo-dist-version` in [dist-workspace.toml](dist-workspace.toml) when validating release plans.
* Have permission to publish `hawkeye` on crates.io, with Cargo registry authentication configured.
* Have push access to the repository and a working Git signing key. [Cargo.toml](Cargo.toml) enables signed release tags named `v{{version}}`.

## Prepare the release PR

Update these files on a branch based on current `main`:

* [hawkeye/Cargo.toml](hawkeye/Cargo.toml): package version.
* [Cargo.toml](Cargo.toml): workspace dependency version for `hawkeye`.
* [Cargo.lock](Cargo.lock): corresponding package version, without unrelated dependency upgrades.
* [README.md](README.md): pre-commit example revision.
* [CHANGELOG.md](CHANGELOG.md): release notes and intended release date, keeping the empty `Unreleased` section for future changes.

Check the repository workflows and package before requesting review:

```sh
cargo x --help
cargo x build --help
cargo x test --help
cargo x lint --help
cargo x build --locked
cargo x test
cargo x lint
cargo publish --dry-run --locked --allow-dirty --package hawkeye
dist generate --check
dist plan
```

Review the planned targets and runner labels, not just whether planning succeeds. Release builds need supported runners even when ordinary CI uses different ones. Update cargo-dist configuration in [dist-workspace.toml](dist-workspace.toml) and regenerate workflows with `dist generate` when needed; do not hand-edit the generated release workflow.

Merge the preparation PR after review and successful CI. If the release date changes, update the changelog before publishing.

## Publish

Use a clean checkout of `main` at the reviewed release commit, with successful CI. Confirm the current version and changelog before executing the release:

```sh
git switch main
git pull --ff-only origin main
git status --short
git log -1 --oneline

# Preview publication without uploading, tagging, or pushing.
cargo release --package hawkeye

# Publish after reviewing the preview.
cargo release --package hawkeye --execute
```

The preparation PR already sets the version. Do not pass `minor` or `patch` here, since that would bump it again. cargo-release publishes the current version to crates.io, creates its signed tag, and pushes to `origin`. The internal `xtask` package is excluded from releases.

The pushed tag starts the Release and Docker workflows. Let the Release workflow create the GitHub Release and upload its assets; no manual GitHub Release creation is needed.

## Verify the release

* Confirm both tag-triggered workflows succeed for the intended commit.
* Check the GitHub Release title and notes, archives for every configured target, checksums, and shell installer.
* Check the version on crates.io and its documentation build on docs.rs.
* Check that the versioned GHCR image contains both Linux amd64 and arm64 manifests.
* Run a published binary and the versioned container with `--version`, and smoke-test header formatting on a disposable project.

## Resume an interrupted release

Inspect crates.io, local and remote tags, and the Actions runs before retrying. Keep the checkout on the original release commit; do not move or overwrite a published version tag.

cargo-release provides separate `publish`, `tag`, and `push` steps. Use `cargo release <step> --package hawkeye` to preview only the missing step, then repeat it with `--execute`. The `publish` step skips already published versions, and the `tag` step skips existing local tags; verify any existing tag points to the intended commit before pushing.

If the tag is already on GitHub, inspect the failed Release or Docker job and rerun it when the failure is transient. Do not republish the crate or create another tag just to restart Actions. A source or workflow correction requires a reviewed fix and a new version rather than replacing an existing release tag.
