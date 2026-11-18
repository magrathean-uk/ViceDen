use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn release_smoke_script_accepts_bootable_tarball() {
    let root = repo_root();
    let temp = std::env::temp_dir().join(format!("viceden-release-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(temp.join("package/viceden/bin")).expect("create fake package bin");

    let fake_binary = temp.join("package/viceden/bin/viceden");
    fs::write(&fake_binary, fake_viceden_server()).expect("write fake viceden binary");
    fs::set_permissions(&fake_binary, fs::Permissions::from_mode(0o755))
        .expect("mark fake binary executable");
    fs::create_dir_all(temp.join("package/viceden/config")).expect("create fake package config");
    fs::create_dir_all(temp.join("package/viceden/systemd")).expect("create fake package systemd");
    fs::write(temp.join("package/viceden/config/default.toml"), "").expect("write default config");
    fs::write(temp.join("package/viceden/config/boards.toml"), "").expect("write boards config");
    fs::create_dir_all(temp.join("package/viceden/static/assets"))
        .expect("create fake static assets dir");
    for asset in [
        "viceden-app.css",
        "viceden-auth.css",
        "viceden-motion.css",
        "viceden-public.css",
        "viceden-ui.js",
    ] {
        fs::write(temp.join("package/viceden/static/assets").join(asset), "")
            .expect("write fake static asset");
    }
    fs::create_dir_all(temp.join("package/viceden/packaging")).expect("create fake packaging dir");
    fs::write(
        temp.join("package/viceden/packaging/viceden.env.example"),
        "VICEDEN_DATABASE__REQUIRE_CONNECTION=true\n",
    )
    .expect("write env example");
    fs::write(temp.join("package/viceden/README.md"), "# ViceDen\n").expect("write readme");
    fs::write(
        temp.join("package/viceden/systemd/viceden.service"),
        [
            "EnvironmentFile=/etc/default/viceden",
            "ExecStart=/opt/sites/viceden/runtime/current/bin/viceden",
            "WorkingDirectory=/opt/sites/viceden/runtime/current",
            "ReadWritePaths=/opt/sites/viceden/data /opt/sites/viceden/logs",
            "",
        ]
        .join("\n"),
    )
    .expect("write systemd unit");

    let tarball = temp.join("viceden-test-linux-x86_64.tar.gz");
    let tar_status = Command::new("tar")
        .args([
            "-C",
            temp.join("package").to_str().unwrap(),
            "-czf",
            tarball.to_str().unwrap(),
            "viceden",
        ])
        .status()
        .expect("create fake tarball");
    assert!(tar_status.success(), "tar should create fake package");

    let smoke_status = Command::new(root.join("scripts/smoke-release.sh"))
        .arg(&tarball)
        .status()
        .expect("run release smoke script");
    assert!(smoke_status.success(), "release smoke script should pass");

    let _ = fs::remove_dir_all(&temp);
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn fake_viceden_server() -> &'static str {
    r#"#!/usr/bin/env bash
set -euo pipefail
bind="${VICEDEN_SERVER__BIND:-127.0.0.1:39137}"
host="${bind%:*}"
port="${bind##*:}"
python3 - "$host" "$port" <<'PY'
import http.server
import sys

host = sys.argv[1]
port = int(sys.argv[2])

class Handler(http.server.BaseHTTPRequestHandler):
    user_handle = "root"
    report_id = "00000000-0000-0000-0000-000000000002"
    report_resolved = False

    def write(self, status, body, content_type="text/html"):
        if isinstance(body, str):
            body = body.encode()
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("set-cookie", "viceden_pre_auth_csrf=csrf; Path=/; HttpOnly; SameSite=Lax")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/api/healthz":
            self.write(200, '{"status":"ok"}', "application/json")
            return
        if self.path in ("/register", "/login"):
            self.write(200, '<input type="hidden" name="csrf" value="csrf" />')
            return
        if self.path == "/profile":
            self.write(200, f'{Handler.user_handle}<input type="hidden" name="csrf" value="csrf" />')
            return
        if self.path == "/notifications":
            self.write(200, "save the raw token now: vdn_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            return
        if self.path == "/api/me":
            self.write(200, '{"handle":"' + Handler.user_handle + '"}', "application/json")
            return
        if self.path == "/admin/reports":
            if Handler.report_resolved:
                self.write(200, 'No reports queued.<input type="hidden" name="csrf" value="csrf" />')
            else:
                self.write(200, f'{Handler.report_id} open <button>Resolve</button><input type="hidden" name="csrf" value="csrf" />')
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        import json
        import urllib.parse

        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length).decode()
        form = urllib.parse.parse_qs(raw)
        if self.path == "/register":
            Handler.user_handle = form.get("handle", ["smoke"])[0]
            self.write(200, Handler.user_handle)
            return
        if self.path == "/login":
            Handler.user_handle = form.get("handle", ["root"])[0]
            self.write(200, Handler.user_handle)
            return
        if self.path == "/profile/pats":
            self.write(200, "pat created")
            return
        if self.path == "/logout":
            self.write(200, "logged out")
            return
        if self.path == "/api/boards/hobbies/threads":
            self.write(200, '{"id":"00000000-0000-0000-0000-000000000003"}', "application/json")
            return
        if self.path.startswith("/api/threads/") and self.path.endswith("/report"):
            Handler.report_resolved = False
            self.write(200, '{"id":"' + Handler.report_id + '"}', "application/json")
            return
        if self.path == f"/admin/reports/{Handler.report_id}/resolve":
            Handler.report_resolved = True
            self.write(200, "resolved")
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, *_args):
        return

http.server.ThreadingHTTPServer((host, port), Handler).serve_forever()
PY
"#
}
