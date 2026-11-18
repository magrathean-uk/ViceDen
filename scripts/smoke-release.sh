#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <release-tarball>" >&2
    exit 2
fi

TARBALL="$1"
if [[ ! -f "$TARBALL" ]]; then
    echo "release tarball not found: $TARBALL" >&2
    exit 2
fi

command -v tar >/dev/null 2>&1 || {
    echo "tar is required" >&2
    exit 2
}
command -v curl >/dev/null 2>&1 || {
    echo "curl is required" >&2
    exit 2
}
command -v grep >/dev/null 2>&1 || {
    echo "grep is required" >&2
    exit 2
}
command -v sed >/dev/null 2>&1 || {
    echo "sed is required" >&2
    exit 2
}

TMP_DIR="$(mktemp -d)"
APP_PID=""
PG_DATA=""
cleanup() {
    if [[ -n "$APP_PID" ]] && kill -0 "$APP_PID" >/dev/null 2>&1; then
        kill "$APP_PID" >/dev/null 2>&1 || true
        wait "$APP_PID" >/dev/null 2>&1 || true
    fi
    if [[ -n "$PG_DATA" ]]; then
        pg_ctl -D "$PG_DATA" -w stop >/dev/null 2>&1 || true
    fi
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

tar -C "$TMP_DIR" -xzf "$TARBALL"

APP_DIR="$TMP_DIR/viceden"
APP_BIN="$APP_DIR/bin/viceden"
APP_LOG="$TMP_DIR/viceden.log"
if [[ -f "$TARBALL.sha256" ]]; then
    command -v sha256sum >/dev/null 2>&1 || {
        echo "sha256sum is required when checksum file exists" >&2
        exit 2
    }
    (
        cd "$(dirname "$TARBALL")"
        sha256sum -c "$(basename "$TARBALL").sha256"
    ) >/dev/null
fi

for required_path in \
    "$APP_BIN" \
    "$APP_DIR/config/default.toml" \
    "$APP_DIR/config/boards.toml" \
    "$APP_DIR/static/assets/viceden-app.css" \
    "$APP_DIR/static/assets/viceden-auth.css" \
    "$APP_DIR/static/assets/viceden-motion.css" \
    "$APP_DIR/static/assets/viceden-public.css" \
    "$APP_DIR/static/assets/viceden-ui.js" \
    "$APP_DIR/packaging/viceden.env.example" \
    "$APP_DIR/README.md" \
    "$APP_DIR/systemd/viceden.service"; do
    if [[ ! -e "$required_path" ]]; then
        echo "release tarball missing ${required_path#"$APP_DIR"/}" >&2
        exit 1
    fi
done

extract_hidden_value() {
    local name="$1"
    sed -n "s/.*name=\"${name}\" value=\"\([^\"]*\)\".*/\1/p" | head -n1
}

require_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"
    if [[ "$haystack" != *"$needle"* ]]; then
        echo "$label missing: $needle" >&2
        exit 1
    fi
}

require_not_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        echo "$label still present: $needle" >&2
        exit 1
    fi
}

if [[ ! -x "$APP_BIN" ]]; then
    echo "release tarball bin/viceden is not executable" >&2
    exit 1
fi

UNIT_FILE="$APP_DIR/systemd/viceden.service"
for required_line in \
    "EnvironmentFile=/etc/default/viceden" \
    "ExecStart=/opt/sites/viceden/runtime/current/bin/viceden" \
    "WorkingDirectory=/opt/sites/viceden/runtime/current" \
    "ReadWritePaths=/opt/sites/viceden/data /opt/sites/viceden/logs"; do
    if ! grep -Fqx "$required_line" "$UNIT_FILE"; then
        echo "release unit missing: $required_line" >&2
        exit 1
    fi
done

pick_port() {
    if command -v python3 >/dev/null 2>&1; then
        python3 - <<'PY'
import socket

with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
        return
    fi
    echo "39137"
}

start_smoke_postgres() {
    command -v initdb >/dev/null 2>&1 || {
        echo "initdb is required unless VICEDEN_SMOKE_DATABASE_URL is set" >&2
        exit 2
    }
    command -v pg_ctl >/dev/null 2>&1 || {
        echo "pg_ctl is required unless VICEDEN_SMOKE_DATABASE_URL is set" >&2
        exit 2
    }
    command -v createdb >/dev/null 2>&1 || {
        echo "createdb is required unless VICEDEN_SMOKE_DATABASE_URL is set" >&2
        exit 2
    }

    local pg_port pg_root db_name user log
    pg_port="$(pick_port)"
    pg_root="$TMP_DIR/postgres"
    PG_DATA="$pg_root/data"
    db_name="viceden_release_smoke_test"
    user="${USER:-postgres}"
    log="$pg_root/postgres.log"
    mkdir -p "$pg_root"
    initdb -D "$PG_DATA" -A trust -U "$user" >/dev/null
    pg_ctl -D "$PG_DATA" -l "$log" -o "-h 127.0.0.1 -p $pg_port" -w start >/dev/null
    createdb -h 127.0.0.1 -p "$pg_port" -U "$user" "$db_name"
    printf 'postgres://%s@127.0.0.1:%s/%s\n' "$user" "$pg_port" "$db_name"
}

PORT="$(pick_port)"
BASE_URL="http://127.0.0.1:$PORT"
APP_DATABASE_URL="${VICEDEN_SMOKE_DATABASE_URL:-${VICEDEN_DATABASE__URL:-}}"
if [[ -z "$APP_DATABASE_URL" ]]; then
    APP_DATABASE_URL="$(start_smoke_postgres)"
fi
FOUNDER_COOKIE_JAR="$TMP_DIR/founder.cookies"
FOUNDER_PASSWORD="SmokeFounderPass1!"
THREAD_TITLE="Smoke thread root"
THREAD_BODY="Smoke body for founder"
PAT_NAME="smoke-api"
REPORT_REASON="smoke report"
REPORT_DETAILS="release smoke"

(
    cd "$APP_DIR"
    VICEDEN_SERVER__BIND="127.0.0.1:$PORT" \
        VICEDEN_DATABASE__URL="$APP_DATABASE_URL" \
        VICEDEN_DATABASE__REQUIRE_CONNECTION=false \
        VICEDEN_AUTH__FOUNDER_PASSWORD="$FOUNDER_PASSWORD" \
        "$APP_BIN"
) >"$APP_LOG" 2>&1 &
APP_PID="$!"

for _ in {1..80}; do
    if ! kill -0 "$APP_PID" >/dev/null 2>&1; then
        echo "release binary exited during smoke" >&2
        sed -n '1,160p' "$APP_LOG" >&2
        exit 1
    fi

    BODY="$(curl -fsS --max-time 2 "http://127.0.0.1:$PORT/api/healthz" 2>/dev/null || true)"
    if [[ "$BODY" == *'"status":"ok"'* ]]; then
        break
    fi
    sleep 0.25
done

if [[ "$BODY" != *'"status":"ok"'* ]]; then
    echo "release binary did not answer /api/healthz" >&2
    sed -n '1,160p' "$APP_LOG" >&2
    exit 1
fi

FOUNDER_LOGIN_PAGE="$(curl -fsS -c "$FOUNDER_COOKIE_JAR" -b "$FOUNDER_COOKIE_JAR" "$BASE_URL/login")"
FOUNDER_LOGIN_CSRF="$(printf '%s' "$FOUNDER_LOGIN_PAGE" | extract_hidden_value csrf | head -n1)"
if [[ -z "$FOUNDER_LOGIN_CSRF" ]]; then
    echo "missing founder login csrf token" >&2
    exit 1
fi
FOUNDER_LOGIN_HTML="$(curl -fsS -c "$FOUNDER_COOKIE_JAR" -b "$FOUNDER_COOKIE_JAR" -L \
    --data-urlencode "handle=root" \
    --data-urlencode "password=$FOUNDER_PASSWORD" \
    --data-urlencode "csrf=$FOUNDER_LOGIN_CSRF" \
    --data-urlencode "return_to=/profile" \
    "$BASE_URL/login")"
require_contains "$FOUNDER_LOGIN_HTML" "root" "founder login response"

PROFILE_HTML="$(curl -fsS -b "$FOUNDER_COOKIE_JAR" "$BASE_URL/profile")"
require_contains "$PROFILE_HTML" "root" "profile page"
FOUNDER_CSRF="$(printf '%s' "$PROFILE_HTML" | extract_hidden_value csrf | head -n1)"
if [[ -z "$FOUNDER_CSRF" ]]; then
    echo "missing founder csrf token" >&2
    exit 1
fi

curl -fsS -b "$FOUNDER_COOKIE_JAR" -c "$FOUNDER_COOKIE_JAR" -L \
    --data-urlencode "csrf=$FOUNDER_CSRF" \
    --data-urlencode "name=$PAT_NAME" \
    "$BASE_URL/profile/pats" >/dev/null

NOTIFICATIONS_HTML="$(curl -fsS -b "$FOUNDER_COOKIE_JAR" "$BASE_URL/notifications")"
PAT_TOKEN="$(printf '%s' "$NOTIFICATIONS_HTML" | sed -n 's/.*save the raw token now: \(vdn_[0-9a-f]\{64\}\).*/\1/p' | head -n1)"
if [[ -z "$PAT_TOKEN" ]]; then
    echo "missing PAT token in notifications" >&2
    sed -n '1,160p' "$APP_LOG" >&2
    exit 1
fi

API_ME="$(curl -fsS -H "Authorization: Bearer $PAT_TOKEN" "$BASE_URL/api/me")"
require_contains "$API_ME" "root" "api me"

THREAD_JSON="$(curl -fsS -X POST "$BASE_URL/api/boards/hobbies/threads" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $PAT_TOKEN" \
    -d "{\"title\":\"$THREAD_TITLE\",\"body\":\"$THREAD_BODY\"}")"
THREAD_ID="$(printf '%s' "$THREAD_JSON" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | head -n1)"
if [[ -z "$THREAD_ID" ]]; then
    echo "missing thread id from api create thread" >&2
    exit 1
fi

REPORT_JSON="$(curl -fsS -X POST "$BASE_URL/api/threads/$THREAD_ID/report" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $PAT_TOKEN" \
    -d "{\"reason\":\"$REPORT_REASON\",\"details\":\"$REPORT_DETAILS\"}")"
REPORT_ID="$(printf '%s' "$REPORT_JSON" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | head -n1)"
if [[ -z "$REPORT_ID" ]]; then
    echo "missing report id from api report" >&2
    exit 1
fi

ADMIN_HTML="$(curl -fsS -b "$FOUNDER_COOKIE_JAR" "$BASE_URL/admin/reports")"
require_contains "$ADMIN_HTML" "$REPORT_ID" "admin report queue"
ADMIN_CSRF="$(printf '%s' "$ADMIN_HTML" | extract_hidden_value csrf | head -n1)"
if [[ -z "$ADMIN_CSRF" ]]; then
    echo "missing admin csrf token" >&2
    exit 1
fi

curl -fsS -b "$FOUNDER_COOKIE_JAR" -c "$FOUNDER_COOKIE_JAR" -L \
    --data-urlencode "csrf=$ADMIN_CSRF" \
    --data-urlencode "resolved=true" \
    "$BASE_URL/admin/reports/$REPORT_ID/resolve" >/dev/null

FINAL_ADMIN_HTML="$(curl -fsS -b "$FOUNDER_COOKIE_JAR" "$BASE_URL/admin/reports")"
require_not_contains "$FINAL_ADMIN_HTML" "$REPORT_ID" "resolved admin report"

echo "release smoke ok"
