use viceden::{config, db};

fn bad_database_settings(require_connection: bool) -> config::DatabaseSettings {
    config::DatabaseSettings {
        url: "postgres://postgres:postgres@127.0.0.1:1/viceden?connect_timeout=1".to_string(),
        require_connection,
        max_connections: 1,
    }
}

#[tokio::test]
async fn database_connection_failure_stops_startup() {
    let error = db::connect_pool(&bad_database_settings(false))
        .await
        .expect_err("database mode should fail startup");

    assert!(error.to_string().contains("database connection failed"));
}

#[tokio::test]
async fn disabled_database_url_fails_startup() {
    let error = db::connect_pool(&config::DatabaseSettings {
        url: "disabled".to_string(),
        require_connection: false,
        max_connections: 1,
    })
    .await
    .expect_err("disabled database URL should fail startup");

    assert!(error.to_string().contains("VICEDEN_DATABASE__URL"));
}

#[tokio::test]
async fn required_database_connection_fails_startup() {
    let error = db::connect_pool(&bad_database_settings(true))
        .await
        .expect_err("required database mode should fail startup");

    assert!(error.to_string().contains("database connection failed"));
}
