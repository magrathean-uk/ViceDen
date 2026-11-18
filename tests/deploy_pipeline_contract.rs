#[test]
fn github_workflow_publishes_rolling_release_assets() {
    let workflow = include_str!("../.github/workflows/deploy.yml");

    for expected in [
        "branches:",
        "- main",
        "RELEASE_TAG: deploy-main",
        "./scripts/release.sh",
        "softprops/action-gh-release@v3",
        "${{ env.RELEASE_DIST }}/*",
    ] {
        assert!(workflow.contains(expected), "workflow missing {expected}");
    }
}

#[test]
fn deploy_files_use_sites_layout_and_dedicated_database() {
    let service = include_str!("../packaging/viceden.service");
    let env = include_str!("../packaging/viceden.env.example");
    let postgres_service = include_str!("../ops/viceden-postgres.service");
    let deploy = include_str!("../deploy.sh");
    let poller = include_str!("../scripts/poll_release.py");
    let backup = include_str!("../scripts/backup-postgres.sh");

    assert!(service.contains("/opt/sites/viceden/runtime/current/bin/viceden"));
    assert!(service.contains("SyslogIdentifier=viceden"));
    assert!(env.contains("127.0.0.1:8091"));
    assert!(env.contains("127.0.0.1:5433"));
    assert!(postgres_service.contains("postgresql@18-main.service"));
    assert!(postgres_service.contains("ExecStart=/bin/true"));
    assert!(!deploy.contains("$PROJECT_DIR/postgres"));
    assert!(deploy.contains("systemctl restart viceden-postgres.service"));
    assert!(poller.contains("deploy-main"));
    assert!(backup.contains("-h \"$PGHOST\""));
    assert!(backup.contains("VICEDEN_POSTGRES_HOST:-127.0.0.1"));
    assert!(backup.contains("-p \"$PGPORT\""));
    assert!(backup.contains("VICEDEN_POSTGRES_PORT:-5433"));

    for forbidden in ["/opt/viceden", "root-database-1", "docker exec", "15432"] {
        assert!(!service.contains(forbidden), "service contains {forbidden}");
        assert!(!env.contains(forbidden), "env contains {forbidden}");
        assert!(!postgres_service.contains(forbidden), "postgres service contains {forbidden}");
        assert!(!backup.contains(forbidden), "backup contains {forbidden}");
    }
}
