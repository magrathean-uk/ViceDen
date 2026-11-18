#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="${VICEDEN_PROJECT_DIR:-/opt/sites/viceden}"
RUNTIME_DIR="$PROJECT_DIR/runtime"
RELEASES_DIR="$RUNTIME_DIR/releases"
DOWNLOADS_DIR="$RUNTIME_DIR/downloads"
CURRENT_LINK="$RUNTIME_DIR/current"
KEEP_RELEASES="${KEEP_RELEASES:-3}"
KEEP_DOWNLOADS="${KEEP_DOWNLOADS:-3}"
HEALTH_URL="${VICEDEN_HEALTH_URL:-http://127.0.0.1:8091/api/healthz}"

ASSET_URL="${1:?missing runtime tarball url}"
ASSET_NAME="${2:?missing runtime tarball name}"
CHECKSUM_URL="${3:?missing checksum url}"
RELEASE_ID="${4:?missing release id}"

LOCK_FILE="/tmp/viceden-deploy.lock"

exec 200>"$LOCK_FILE"
flock -n 200 || {
  echo "deploy already running"
  exit 1
}

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

mkdir -p "$RELEASES_DIR" "$DOWNLOADS_DIR" "$PROJECT_DIR/scripts" "$PROJECT_DIR/ops"
if id viceden >/dev/null 2>&1; then
  install -d -o viceden -g viceden "$PROJECT_DIR/data" "$PROJECT_DIR/logs"
else
  install -d "$PROJECT_DIR/data" "$PROJECT_DIR/logs"
fi

RELEASE_DIR="$RELEASES_DIR/$RELEASE_ID"
ARCHIVE_TMP="$TMP_DIR/$ASSET_NAME"
CHECKSUM_TMP="$TMP_DIR/SHA256SUMS"
ARCHIVE_CACHE="$DOWNLOADS_DIR/$RELEASE_ID-$ASSET_NAME"
CHECKSUM_CACHE="$DOWNLOADS_DIR/$RELEASE_ID-SHA256SUMS"

echo "starting viceden deploy for $RELEASE_ID"

curl_download() {
  local url="$1"
  local output="$2"
  if [ -n "${GITHUB_TOKEN:-}" ] && [[ "$url" == https://api.github.com/* ]]; then
    curl -fsSL \
      -H "Authorization: Bearer $GITHUB_TOKEN" \
      -H "Accept: application/octet-stream" \
      "$url" -o "$output"
  else
    curl -fsSL "$url" -o "$output"
  fi
}

curl_download "$ASSET_URL" "$ARCHIVE_TMP"
curl_download "$CHECKSUM_URL" "$CHECKSUM_TMP"

(
  cd "$TMP_DIR"
  grep -F "  $ASSET_NAME" SHA256SUMS | sha256sum -c -
)

cp "$ARCHIVE_TMP" "$ARCHIVE_CACHE"
cp "$CHECKSUM_TMP" "$CHECKSUM_CACHE"

rm -rf "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR"
tar -xzf "$ARCHIVE_TMP" -C "$TMP_DIR"
if [[ ! -d "$TMP_DIR/viceden" ]]; then
  echo "release archive missing viceden root" >&2
  exit 1
fi
cp -a "$TMP_DIR/viceden/." "$RELEASE_DIR/"

chmod 755 "$RELEASE_DIR/bin/viceden"
chmod 755 "$RELEASE_DIR/deploy.sh" "$RELEASE_DIR/scripts/"*.py "$RELEASE_DIR/scripts/"*.sh

ln -sfn "releases/$RELEASE_ID" "$CURRENT_LINK"

install -m 755 "$RELEASE_DIR/deploy.sh" "$PROJECT_DIR/deploy.sh"
install -m 755 "$RELEASE_DIR/scripts/poll_release.py" "$PROJECT_DIR/scripts/poll_release.py"
install -m 755 "$RELEASE_DIR/scripts/backup-postgres.sh" "$PROJECT_DIR/scripts/backup-postgres.sh"
cp -a "$RELEASE_DIR/ops/." "$PROJECT_DIR/ops/"

install -m 644 "$RELEASE_DIR/systemd/viceden.service" /etc/systemd/system/viceden.service
install -m 644 "$PROJECT_DIR/ops/viceden-deploy-poll.service" /etc/systemd/system/viceden-deploy-poll.service
install -m 644 "$PROJECT_DIR/ops/viceden-deploy-poll.timer" /etc/systemd/system/viceden-deploy-poll.timer
install -m 644 "$PROJECT_DIR/ops/viceden-postgres.service" /etc/systemd/system/viceden-postgres.service
install -m 644 "$PROJECT_DIR/ops/viceden-backup.service" /etc/systemd/system/viceden-backup.service
install -m 644 "$PROJECT_DIR/ops/viceden-backup.timer" /etc/systemd/system/viceden-backup.timer
systemctl daemon-reload

systemctl restart viceden-postgres.service
systemctl restart viceden.service

for _ in {1..40}; do
  if curl -fsS "$HEALTH_URL" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

curl -fsS "$HEALTH_URL" >/dev/null

ls -1dt "$RELEASES_DIR"/* 2>/dev/null | tail -n +"$((KEEP_RELEASES + 1))" | xargs -r rm -rf
ls -1t "$DOWNLOADS_DIR"/* 2>/dev/null | tail -n +"$((KEEP_DOWNLOADS * 2 + 1))" | xargs -r rm -f

echo "viceden deploy finished for $RELEASE_ID"
