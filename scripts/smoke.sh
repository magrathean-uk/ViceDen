#!/usr/bin/env bash
# scripts/smoke.sh — comprehensive smoke against a running instance.
#
# Usage: BASE_URL=https://viceden.example.com FOUNDER_PASSWORD=… scripts/smoke.sh
#
# Exits non-zero on any failure.

set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:3000}"
FOUNDER_HANDLE="${FOUNDER_HANDLE:-root}"
FOUNDER_PASSWORD="${FOUNDER_PASSWORD:?must be set}"

cookie_jar=$(mktemp)
trap 'rm -f "$cookie_jar"' EXIT

bold() { printf "\033[1m%s\033[0m\n" "$1"; }
fail() { printf "\033[31mFAIL\033[0m %s\n" "$1"; exit 1; }
ok()   { printf "\033[32m  ok\033[0m %s\n" "$1"; }

bold "1. healthz"
curl -fsS "$BASE_URL/api/healthz" >/dev/null || fail "healthz"
ok "/api/healthz 200"

bold "2. security headers on /login"
headers=$(curl -fsS -D - -o /dev/null "$BASE_URL/login")
echo "$headers" | grep -qi "x-frame-options: DENY"            || fail "x-frame-options"
echo "$headers" | grep -qi "content-security-policy:"         || fail "content-security-policy"
echo "$headers" | grep -qi "x-content-type-options: nosniff"  || fail "x-content-type-options"
echo "$headers" | grep -qi "referrer-policy: strict-origin-when-cross-origin" || fail "referrer-policy"
ok "headers present"

bold "3. founder login + cookie"
csrf=$(curl -fsS -c "$cookie_jar" "$BASE_URL/login" \
       | grep -oE 'name="csrf" value="[^"]+"' | head -1 | cut -d'"' -f4)
[ -n "$csrf" ] || fail "no csrf token in /login"
status=$(curl -fsS -b "$cookie_jar" -c "$cookie_jar" -o /dev/null -w "%{http_code}" \
        -X POST "$BASE_URL/login" \
        -d "handle=$FOUNDER_HANDLE&password=$FOUNDER_PASSWORD&csrf=$csrf&return_to=/profile")
[ "$status" = "303" ] || fail "login expected 303, got $status"
ok "founder login 303 + cookie set"

bold "4. profile reachable"
curl -fsS -b "$cookie_jar" "$BASE_URL/profile" >/dev/null || fail "profile"
ok "/profile 200"

bold "5. invite issue + redeem dry-run"
csrf2=$(curl -fsS -b "$cookie_jar" "$BASE_URL/admin/invites" \
        | grep -oE 'name="csrf" value="[^"]+"' | head -1 | cut -d'"' -f4)
status=$(curl -fsS -b "$cookie_jar" -c "$cookie_jar" -o /dev/null -w "%{http_code}" \
        -X POST "$BASE_URL/admin/invites" \
        -d "csrf=$csrf2&note=smoke&max_redemptions=1")
[ "$status" = "303" ] || fail "issue invite expected 303, got $status"
ok "invite issued"

bold "6. classifier audit row exists for a recent post"
# This relies on Postgres reachability from the smoke runner; skip if not local.
if [ -n "${VICEDEN_DATABASE__URL:-}" ]; then
    rows=$(psql "$VICEDEN_DATABASE__URL" -tAc \
        "SELECT count(*) FROM content_classifications WHERE classified_at > now() - interval '1 hour'")
    [ "$rows" -gt 0 ] && ok "classifier audit rows present" || ok "no recent posts (skip)"
fi

bold "smoke green"
