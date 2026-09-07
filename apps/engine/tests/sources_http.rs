//! In-process HTTP contract tests for the sources host router.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use electric_circuits_engine::config::Config;
use electric_circuits_engine::ds::DsClient;
use electric_circuits_engine::engine::Engine;
use electric_circuits_engine::sources::{SourceRow, SourcesSupervisor};
use tower::ServiceExt;

fn config(file: &str) -> Config {
    Config::resolve(|name| match name {
        "ELECTRIC_CIRCUITS_SOURCES_MODE" => Some("file".into()),
        "ELECTRIC_CIRCUITS_SOURCES_FILE" => Some(file.into()),
        "ELECTRIC_CIRCUITS_SOURCES_POLL_SECS" => Some("1".into()),
        _ => None,
    })
    .expect("valid sources test config")
}

fn row(revision: i64) -> SourceRow {
    SourceRow {
        source_id: "alpha".into(),
        plugin: "pgoutput".into(),
        database_secret: "env:ALPHA_DATABASE_URL".into(),
        slot: "alpha_slot".into(),
        publication: "alpha_slot_pub".into(),
        tables: vec!["public.thread_messages".into()],
        revision,
        updated_at: "2026-09-07T00:00:00Z".into(),
    }
}

async fn body(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn sources_admin_and_proxy_routes_have_the_declared_contract() {
    let file = std::env::temp_dir().join(format!("circuits-sources-http-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, serde_json::to_vec(&vec![row(7)]).unwrap()).unwrap();
    electric_circuits_engine::config::set_globals("sources-http", "sources-http", None, Some("control-secret"));

    let engine = Engine::new_for_in_process_test(DsClient::new_for_in_process_test("http://127.0.0.1:1"));
    let supervisor = SourcesSupervisor::with_test_source(config(file.to_str().unwrap()), row(7), engine).await.unwrap();
    let app = supervisor.router();

    let unauthorized = app
        .clone()
        .oneshot(
            Request::post("/admin/refresh")
                .header("authorization", "Bearer gateway-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let body_rejected = app
        .clone()
        .oneshot(
            Request::post("/admin/refresh")
                .header("authorization", "Bearer control-secret")
                .body(Body::from("not allowed"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_rejected.status(), StatusCode::BAD_REQUEST);

    let refreshed = app
        .clone()
        .oneshot(
            Request::post("/admin/refresh")
                .header("authorization", "Bearer control-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refreshed.status(), StatusCode::OK);
    assert_eq!(body(refreshed).await["revision"], 7);

    let ready = app.clone().oneshot(Request::get("/ready").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(ready.status(), StatusCode::OK);

    let sources = app.clone().oneshot(Request::get("/sources").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(sources.status(), StatusCode::OK);
    let sources = body(sources).await;
    assert_eq!(sources[0]["source_id"], "alpha");
    assert_eq!(sources[0]["revision"], 7);
    assert_eq!(sources[0]["ready"], true);
    assert!(sources[0]["error"].is_null());

    let status = app.clone().oneshot(Request::get("/sources/alpha/status").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status = body(status).await;
    assert_eq!(status["revision"], 7);
    assert_eq!(status["ready"], true);
    assert_eq!(status["readiness"], "active");
    assert_eq!(status["changes"]["route"], "/sources/alpha/changes");

    let proxied = app
        .oneshot(Request::get("/sources/alpha/v1/shape?table=thread_messages").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_ne!(proxied.status(), StatusCode::NOT_FOUND, "the rewritten request must reach the source router");

    supervisor.shutdown_token().begin();
    supervisor.shutdown_all().await;
    std::fs::remove_file(file).unwrap();
}

#[tokio::test]
async fn source_scoped_admin_routes_are_not_found() {
    let file = std::env::temp_dir().join(format!("circuits-sources-admin-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, serde_json::to_vec(&vec![row(7)]).unwrap()).unwrap();
    electric_circuits_engine::config::set_globals(
        "sources-http-admin",
        "sources-http-admin",
        None,
        Some("control-secret"),
    );

    let engine = Engine::new_for_in_process_test(DsClient::new_for_in_process_test("http://127.0.0.1:1"));
    let supervisor = SourcesSupervisor::with_test_source(config(file.to_str().unwrap()), row(7), engine).await.unwrap();
    let app = supervisor.router();

    for uri in [
        "/sources/alpha/_admin/control-admission/close",
        "/sources/alpha/_admin/control-admission/open",
        "/sources/alpha/_admin/deployment/promote",
        "/sources/alpha/_admin/deployment/quiesce",
        "/sources/alpha/_admin/deployment/status",
        "/sources/alpha/_admin",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("authorization", "Bearer control-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri} must not be a source-scoped admin route");
    }

    supervisor.shutdown_token().begin();
    supervisor.shutdown_all().await;
    std::fs::remove_file(file).unwrap();
}

#[tokio::test]
async fn source_scoped_epoch_reset_is_not_found() {
    let file = std::env::temp_dir().join(format!("circuits-sources-epoch-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, serde_json::to_vec(&vec![row(7)]).unwrap()).unwrap();
    electric_circuits_engine::config::set_globals(
        "sources-http-epoch",
        "sources-http-epoch",
        None,
        Some("control-secret"),
    );

    let engine = Engine::new_for_in_process_test(DsClient::new_for_in_process_test("http://127.0.0.1:1"));
    let supervisor = SourcesSupervisor::with_test_source(config(file.to_str().unwrap()), row(7), engine).await.unwrap();
    let app = supervisor.router();

    let before = app.clone().oneshot(Request::get("/sources/alpha/status").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(before.status(), StatusCode::OK);
    let before = body(before).await;

    for (method, uri) in [("POST", "/sources/alpha/epoch/reset"), ("GET", "/sources/alpha/epoch/reset")] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", "Bearer control-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {uri} must not be a source-scoped admin route");
    }

    let after = app.clone().oneshot(Request::get("/sources/alpha/status").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    assert_eq!(body(after).await, before, "source-scoped epoch reset must change nothing");

    supervisor.shutdown_token().begin();
    supervisor.shutdown_all().await;
    std::fs::remove_file(file).unwrap();
}

#[tokio::test]
async fn concurrent_refreshes_serialize_and_return_the_same_revision() {
    let file = std::env::temp_dir().join(format!("circuits-sources-refresh-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&file, serde_json::to_vec(&vec![row(7)]).unwrap()).unwrap();
    electric_circuits_engine::config::set_globals(
        "sources-http-refresh",
        "sources-http-refresh",
        None,
        Some("control-secret"),
    );

    let engine = Engine::new_for_in_process_test(DsClient::new_for_in_process_test("http://127.0.0.1:1"));
    let supervisor = SourcesSupervisor::with_test_source(config(file.to_str().unwrap()), row(7), engine).await.unwrap();
    let app = supervisor.router();

    let refresh = || {
        app.clone().oneshot(
            Request::post("/admin/refresh")
                .header("authorization", "Bearer control-secret")
                .body(Body::empty())
                .unwrap(),
        )
    };
    let (first, second) = tokio::join!(refresh(), refresh());
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(body(first).await["revision"], 7);
    assert_eq!(body(second).await["revision"], 7);

    supervisor.shutdown_token().begin();
    supervisor.shutdown_all().await;
    std::fs::remove_file(file).unwrap();
}

#[tokio::test]
async fn refresh_retries_an_unchanged_failed_source() {
    let file = std::env::temp_dir().join(format!("circuits-sources-retry-{}.json", uuid::Uuid::new_v4()));
    let failed = row(7);
    std::fs::write(&file, serde_json::to_vec(&vec![failed.clone()]).unwrap()).unwrap();
    electric_circuits_engine::config::set_globals(
        "sources-http-retry",
        "sources-http-retry",
        None,
        Some("control-secret"),
    );

    let supervisor =
        SourcesSupervisor::with_test_failed_source(config(file.to_str().unwrap()), failed, "injected-not-retried")
            .await
            .unwrap();
    let app = supervisor.router();

    let before = app.clone().oneshot(Request::get("/sources").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(body(before).await[0]["error"], "injected-not-retried");

    let refreshed = app
        .clone()
        .oneshot(
            Request::post("/admin/refresh")
                .header("authorization", "Bearer control-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refreshed.status(), StatusCode::OK);

    let after = app.oneshot(Request::get("/sources").body(Body::empty()).unwrap()).await.unwrap();
    let after = body(after).await;
    assert_eq!(after[0]["source_id"], "alpha");
    assert_eq!(after[0]["ready"], false);
    assert_ne!(after[0]["error"], "injected-not-retried", "an explicit refresh must retry the failed start");
    let error = after[0]["error"].as_str().unwrap_or_default();
    assert!(!error.contains("ALPHA_DATABASE_URL"), "{error}");
    assert!(!error.contains("postgres://"), "{error}");

    supervisor.shutdown_token().begin();
    supervisor.shutdown_all().await;
    std::fs::remove_file(file).unwrap();
}
