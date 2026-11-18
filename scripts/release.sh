#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "${VICEDEN_RELEASE_ALLOW_DIRTY_TREE:-false}" != "true" ]]; then
    if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
        echo "dirty tree; set VICEDEN_RELEASE_ALLOW_DIRTY_TREE=true to override" >&2
        git status --short >&2
        exit 2
    fi
fi

CARGO_METADATA="$(cargo metadata --format-version 1 --no-deps)"
CARGO_TARGET_DIR="$(printf '%s' "$CARGO_METADATA" | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
PACKAGE_VERSION="$(
    printf '%s' "$CARGO_METADATA" \
        | sed -n 's/.*"packages":\[{"name":"viceden","version":"\([^"]*\)".*/\1/p'
)"
if [[ -z "$PACKAGE_VERSION" ]]; then
    echo "could not read package version from Cargo metadata" >&2
    exit 2
fi
COMMIT_SHA="$(git rev-parse HEAD)"
RELEASE_ID="$(git rev-parse --short=16 HEAD)"
VERSION="${PACKAGE_VERSION}+git.${RELEASE_ID}"
DEB_VERSION="$VERSION"
TARGET_PLAN="$(./scripts/release-target.sh "$CARGO_TARGET_DIR")" || exit $?
eval "$TARGET_PLAN"

install_ops_files() {
    local destination="$1"
    local ops_files=()

    while IFS= read -r -d '' file; do
        ops_files+=("$file")
    done < <(
        find ops -maxdepth 1 -type f \
            \( -name '*.service' -o -name '*.timer' -o -name '*.example' -o -name '*.yml' -o -name '*.conf' \) \
            -print0 | sort -z
    )

    if [[ "${#ops_files[@]}" -eq 0 ]]; then
        echo "no ops files found to package" >&2
        exit 2
    fi

    install -m 644 "${ops_files[@]}" "$destination"
}

if [[ "${VICEDEN_RELEASE_PLAN_ONLY:-false}" == "true" ]]; then
    printf '%s\n' "$TARGET_PLAN"
    exit 0
fi

./scripts/check.sh
if [[ -n "$RELEASE_CARGO_TARGET" ]]; then
    rustup target add "$RELEASE_CARGO_TARGET"
    cargo build --release --locked --target "$RELEASE_CARGO_TARGET"
else
    cargo build --release --locked
fi

RELEASE_BIN="$RELEASE_DIR/viceden"
STAGING_ROOT="$RELEASE_DIR/package/viceden"
TARBALL="$RELEASE_DIR/viceden-${VERSION}-${RELEASE_PLATFORM}.tar.gz"
DEB_STAGING="$RELEASE_DIR/package/deb/viceden_${DEB_VERSION}_amd64"
DEB_PACKAGE="$RELEASE_DIR/viceden_${DEB_VERSION}_amd64.deb"
DIST_DIR="$RELEASE_DIR/dist"
DIST_TARBALL="$DIST_DIR/viceden-linux-x86_64.tar.gz"
DIST_DEB="$DIST_DIR/viceden_amd64.deb"

rm -rf "$(dirname "$STAGING_ROOT")" "$TARBALL" "$TARBALL.sha256" "$DEB_PACKAGE" "$DEB_PACKAGE.sha256" "$DIST_DIR"
mkdir -p \
    "$STAGING_ROOT/bin" \
    "$STAGING_ROOT/config" \
    "$STAGING_ROOT/static/assets" \
    "$STAGING_ROOT/systemd" \
    "$STAGING_ROOT/packaging" \
    "$STAGING_ROOT/ops" \
    "$STAGING_ROOT/scripts" \
    "$DIST_DIR"
install -m 755 "$RELEASE_BIN" "$STAGING_ROOT/bin/viceden"
install -m 644 config/default.toml "$STAGING_ROOT/config/default.toml"
install -m 644 config/boards.toml "$STAGING_ROOT/config/boards.toml"
install -m 644 static/assets/* "$STAGING_ROOT/static/assets/"
install -m 644 packaging/viceden.env.example "$STAGING_ROOT/packaging/viceden.env.example"
install -m 644 README.md "$STAGING_ROOT/README.md"
install -m 644 packaging/viceden.service "$STAGING_ROOT/systemd/viceden.service"
install -m 755 deploy.sh "$STAGING_ROOT/deploy.sh"
install -m 755 scripts/poll_release.py "$STAGING_ROOT/scripts/poll_release.py"
install -m 755 scripts/backup-postgres.sh "$STAGING_ROOT/scripts/backup-postgres.sh"
install_ops_files "$STAGING_ROOT/ops/"
install -m 644 docs/development/debian-deploy-quickstart.md "$STAGING_ROOT/DEPLOY.md"

tar -C "$(dirname "$STAGING_ROOT")" -czf "$TARBALL" "$(basename "$STAGING_ROOT")"
sha256sum "$TARBALL" > "$TARBALL.sha256"
./scripts/smoke-release.sh "$TARBALL"

if command -v dpkg-deb >/dev/null 2>&1; then
    mkdir -p \
        "$DEB_STAGING/DEBIAN" \
        "$DEB_STAGING/opt/sites/viceden/bin" \
        "$DEB_STAGING/opt/sites/viceden/config" \
        "$DEB_STAGING/opt/sites/viceden/static/assets" \
        "$DEB_STAGING/opt/sites/viceden/packaging" \
        "$DEB_STAGING/opt/sites/viceden/ops" \
        "$DEB_STAGING/opt/sites/viceden/scripts" \
        "$DEB_STAGING/opt/sites/viceden/data" \
        "$DEB_STAGING/opt/sites/viceden/logs" \
        "$DEB_STAGING/usr/lib/systemd/system"

    cat > "$DEB_STAGING/DEBIAN/control" <<EOF
Package: viceden
Version: $DEB_VERSION
Section: web
Priority: optional
Architecture: amd64
Maintainer: Magrathean UK Ltd.
Description: ViceDen forum service
EOF

    install -m 755 "$RELEASE_BIN" "$DEB_STAGING/opt/sites/viceden/bin/viceden"
    install -m 644 config/default.toml "$DEB_STAGING/opt/sites/viceden/config/default.toml"
    install -m 644 config/boards.toml "$DEB_STAGING/opt/sites/viceden/config/boards.toml"
    install -m 644 static/assets/* "$DEB_STAGING/opt/sites/viceden/static/assets/"
    install -m 644 packaging/viceden.env.example \
        "$DEB_STAGING/opt/sites/viceden/packaging/viceden.env.example"
    install -m 644 README.md "$DEB_STAGING/opt/sites/viceden/README.md"
    install -m 755 deploy.sh "$DEB_STAGING/opt/sites/viceden/deploy.sh"
    install -m 755 scripts/poll_release.py "$DEB_STAGING/opt/sites/viceden/scripts/poll_release.py"
    install -m 755 scripts/backup-postgres.sh "$DEB_STAGING/opt/sites/viceden/scripts/backup-postgres.sh"
    install_ops_files "$DEB_STAGING/opt/sites/viceden/ops/"
    install -m 644 packaging/viceden.service \
        "$DEB_STAGING/usr/lib/systemd/system/viceden.service"

    dpkg-deb --build "$DEB_STAGING" "$DEB_PACKAGE"
    sha256sum "$DEB_PACKAGE" > "$DEB_PACKAGE.sha256"
    echo "Debian package: $DEB_PACKAGE"
    echo "Debian checksum: $DEB_PACKAGE.sha256"
else
    echo "dpkg-deb not found; skipped .deb build"
fi

cp "$TARBALL" "$DIST_TARBALL"
sha256sum "$DIST_TARBALL" > "$DIST_TARBALL.sha256"
if [[ -f "$DEB_PACKAGE" ]]; then
    cp "$DEB_PACKAGE" "$DIST_DEB"
    sha256sum "$DIST_DEB" > "$DIST_DEB.sha256"
fi

(
    cd "$DIST_DIR"
    if [[ -f viceden_amd64.deb ]]; then
        sha256sum viceden-linux-x86_64.tar.gz viceden_amd64.deb > SHA256SUMS
    else
        sha256sum viceden-linux-x86_64.tar.gz > SHA256SUMS
    fi
)

TARBALL_SHA="$(cut -d ' ' -f1 "$DIST_TARBALL.sha256")"
DEB_SHA=""
if [[ -f "$DIST_DEB.sha256" ]]; then
    DEB_SHA="$(cut -d ' ' -f1 "$DIST_DEB.sha256")"
fi
cat > "$DIST_DIR/manifest.json" <<EOF
{
  "name": "viceden",
  "version": "$VERSION",
  "commit": "$COMMIT_SHA",
  "release_id": "$RELEASE_ID",
  "platform": "$RELEASE_PLATFORM",
  "runtime_tarball": "viceden-linux-x86_64.tar.gz",
  "runtime_tarball_sha256": "$TARBALL_SHA",
  "debian_package": "viceden_amd64.deb",
  "debian_package_sha256": "$DEB_SHA",
  "checksum_file": "SHA256SUMS",
  "deploy_root": "/opt/sites/viceden"
}
EOF

echo "Release tarball: $TARBALL"
echo "Checksum: $TARBALL.sha256"
echo "Deploy dist: $DIST_DIR"
