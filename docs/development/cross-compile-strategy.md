# Linux release builds

A native macOS build produces a macOS binary. It cannot be deployed as a Linux executable.

## Release builder

`scripts/release.sh` delegates platform selection to `scripts/release-target.sh`. Release packaging requires Linux x86_64 because `scripts/smoke-release.sh` boots the packaged binary. The existing deploy workflow supplies a Linux builder.

On that builder, `VICEDEN_RELEASE_TARGET` can select `x86_64-unknown-linux-gnu` or `x86_64-unknown-linux-musl`. The builder must have the matching linker and runtime prerequisites. Packaging rejects a dirty source tree unless its explicit dirty-tree override is set.

## Cross-build helper

```sh
./scripts/build-linux-musl.sh
```

This helper builds for `x86_64-unknown-linux-musl`, but currently sources a developer-specific absolute environment-helper path unconditionally. It is not portable as shipped. Inspect that prerequisite and the linker setup before choosing this command; source changes to make it portable need separate review. A cross-build alone is not a verified release package: package and smoke-test the result on Linux before deployment.

See [quality gates](test-quality-gates.md) and [deployment](debian-deploy-quickstart.md). Do not compile on the production host as a shortcut around the release workflow.
