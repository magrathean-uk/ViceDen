#[test]
fn api_spec_documents_public_beta_routes() {
    let spec = include_str!("../docs/reference/api-v1-spec.md");

    for route in [
        "GET /api/healthz",
        "GET /api/boards",
        "GET /api/boards/{slug}/threads",
        "POST /api/boards/{slug}/threads",
        "POST /api/boards/{slug}/subscription",
        "GET /api/threads/{id}/posts",
        "POST /api/threads/{id}/posts",
        "POST /api/threads/{id}/save",
        "POST /api/threads/{id}/hide",
        "POST /api/threads/{id}/report",
        "POST /api/posts/{id}/vote",
        "POST /api/posts/{id}/report",
        "GET /api/me",
        "PATCH /api/me/settings",
        "GET /api/me/notifications",
        "PATCH /api/me/notifications/{id}",
        "POST /api/me/notifications/read-all",
        "GET /api/me/pats",
        "POST /api/me/pats",
        "DELETE /api/me/pats/{id}",
        "GET /api/admin/users",
        "POST /api/admin/users/{id}/wipe",
        "POST /api/admin/users/{id}/delete",
        "GET /api/admin/reports",
        "PATCH /api/admin/reports/{id}",
        "POST /api/admin/moderation/{target}/{id}",
    ] {
        assert!(spec.contains(route), "API spec missing {route}");
    }

    assert!(!spec.contains("/api/users/{id}"));
    assert!(!spec.contains("POST /api/threads\n"));
}
