use std::{path::Path, process::Command};

#[test]
fn release_target_rejects_non_linux_host_without_explicit_target() {
    let output = release_target_command()
        .env_remove("VICEDEN_RELEASE_TARGET")
        .env("VICEDEN_RELEASE_HOST_OS", "Darwin")
        .env("VICEDEN_RELEASE_HOST_ARCH", "arm64")
        .arg("/tmp/viceden-target")
        .output()
        .expect("run release target helper");

    assert!(!output.status.success(), "non-linux host should be rejected");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("x86_64 Linux VM or CI runner"),
        "error should explain where release packaging must run"
    );
}

#[test]
fn release_target_accepts_linux_x86_64_host() {
    let output = release_target_command()
        .env_remove("VICEDEN_RELEASE_TARGET")
        .env("VICEDEN_RELEASE_HOST_OS", "Linux")
        .env("VICEDEN_RELEASE_HOST_ARCH", "x86_64")
        .arg("/tmp/viceden-target")
        .output()
        .expect("run release target helper");

    assert!(output.status.success(), "linux x86_64 host should be accepted");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("RELEASE_PLATFORM=linux-x86_64"));
    assert!(stdout.contains("RELEASE_DIR=/tmp/viceden-target/release"));
    assert!(stdout.contains("RELEASE_CARGO_TARGET=''"));
}

#[test]
fn release_target_accepts_explicit_linux_x86_64_target_on_linux_builder() {
    let output = release_target_command()
        .env("VICEDEN_RELEASE_TARGET", "x86_64-unknown-linux-musl")
        .env("VICEDEN_RELEASE_HOST_OS", "Linux")
        .env("VICEDEN_RELEASE_HOST_ARCH", "x86_64")
        .arg("/tmp/viceden-target")
        .output()
        .expect("run release target helper");

    assert!(output.status.success(), "explicit linux x86_64 target should be accepted");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("RELEASE_PLATFORM=linux-x86_64"));
    assert!(stdout.contains("RELEASE_DIR=/tmp/viceden-target/x86_64-unknown-linux-musl/release"));
    assert!(stdout.contains("RELEASE_CARGO_TARGET=x86_64-unknown-linux-musl"));
}

#[test]
fn release_target_rejects_explicit_target_on_non_linux_builder() {
    let output = release_target_command()
        .env("VICEDEN_RELEASE_TARGET", "x86_64-unknown-linux-musl")
        .env("VICEDEN_RELEASE_HOST_OS", "Darwin")
        .env("VICEDEN_RELEASE_HOST_ARCH", "arm64")
        .arg("/tmp/viceden-target")
        .output()
        .expect("run release target helper");

    assert!(!output.status.success(), "release helper should reject targets it cannot boot-smoke");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Linux x86_64 builder"));
}

#[test]
fn release_script_stops_when_target_plan_fails() {
    let output = Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/release.sh"))
        .env_remove("VICEDEN_RELEASE_TARGET")
        .env("VICEDEN_RELEASE_HOST_OS", "Darwin")
        .env("VICEDEN_RELEASE_HOST_ARCH", "arm64")
        .env("VICEDEN_RELEASE_PLAN_ONLY", "true")
        .env("VICEDEN_RELEASE_ALLOW_DIRTY_TREE", "true")
        .output()
        .expect("run release script");

    assert!(!output.status.success(), "release script should fail before build steps");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("release packaging requires Linux x86_64")
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Finished `"), "release script should stop before Cargo runs");
    assert!(!stdout.contains("Diff in "), "release script should stop before format checks");
}

#[test]
fn release_script_uses_debian_safe_numeric_version() {
    let script = include_str!("../scripts/release.sh");

    assert!(
        script.contains("PACKAGE_VERSION="),
        "release script should derive package version from Cargo metadata"
    );
    assert!(
        script.contains("DEB_VERSION=\"$VERSION\""),
        "Debian package version should reuse the numeric release version"
    );
    assert!(
        !script.contains("DEB_VERSION=\"$(printf '%s' \"$VERSION\""),
        "Debian package version must not be raw git describe output"
    );
}

#[test]
fn release_script_packages_static_assets() {
    let script = include_str!("../scripts/release.sh");

    assert!(
        script.contains("\"$STAGING_ROOT/static/assets\""),
        "runtime tarball should contain static assets"
    );
    assert!(
        script.contains("install -m 644 static/assets/* \"$STAGING_ROOT/static/assets/\""),
        "runtime tarball should install static assets"
    );
    assert!(
        script.contains("\"$DEB_STAGING/opt/sites/viceden/static/assets\""),
        "deb should contain static assets"
    );
}

#[test]
fn release_script_allows_optional_ops_file_groups() {
    let script = include_str!("../scripts/release.sh");

    assert!(script.contains("install_ops_files"), "ops packaging should go through the helper");
    assert!(
        script.contains("find ops -maxdepth 1 -type f"),
        "ops packaging should discover existing files"
    );
    assert!(
        !script.contains("install -m 644 ops/*.service"),
        "unmatched ops globs should not break release packaging"
    );
}

fn release_target_command() -> Command {
    Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/release-target.sh"))
}
