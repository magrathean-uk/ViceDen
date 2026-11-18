BUILD_ENV ?= ../build-env.sh

.PHONY: check fmt clippy test migrate-test run release package build-linux-musl

check:
	@VICEDEN_BUILD_ENV="$(BUILD_ENV)" ./scripts/check.sh

# G1: migration replay regression — applies every migration to a fresh
# database, then runs the lib tests + the #[ignore] integration tests
# against the resulting schema. Catches accidental destructive
# migrations and shape drifts that pure-build gates miss.
#
# Requires `VICEDEN_DATABASE__URL` pointing at a Postgres the caller is
# willing to wipe (use a throwaway DB or a `_test` suffix).
migrate-test:
	@if [ -f "$(BUILD_ENV)" ]; then . "$(BUILD_ENV)"; fi; ./scripts/migrate-test.sh

fmt:
	@if [ -f "$(BUILD_ENV)" ]; then . "$(BUILD_ENV)"; fi; cargo fmt

clippy:
	@if [ -f "$(BUILD_ENV)" ]; then . "$(BUILD_ENV)"; fi; cargo clippy --all-targets --all-features -- -D warnings

test:
	@if [ -f "$(BUILD_ENV)" ]; then . "$(BUILD_ENV)"; fi; cargo test

run:
	@if [ -f "$(BUILD_ENV)" ]; then . "$(BUILD_ENV)"; fi; cargo run

release: package

package:
	@if [ -f "$(BUILD_ENV)" ]; then . "$(BUILD_ENV)"; fi; ./scripts/release.sh

build-linux-musl:
	@if [ -f "$(BUILD_ENV)" ]; then . "$(BUILD_ENV)"; fi; ./scripts/build-linux-musl.sh
