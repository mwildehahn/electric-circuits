//! Black-box `/v1/shape` handling of terminal and transient backing-stream reads.
//!
//! A closed or deleted shape stream is not a retryable server fault: the Electric client must
//! discard its handle and take a fresh snapshot (`409` + `must-refetch`). A transient durable
//! streams failure remains a `500`, so the client retries the same handle instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use electric_circuits_engine::ds::DsClient;
use electric_circuits_engine::engine::Engine;
use electric_circuits_engine::http::router;
use electric_circuits_engine::schema::Schema;
use tower::ServiceExt;

/// 0 = ordinary stream, 1 = closed, 2 = deleted (404), 3 = gone (410), 4 = transient (503).
#[derive(Clone, Default)]
struct FakeDs {
    read_mode: Arc<AtomicU8>,
}

async fn ds_handler(axum::extract::State(ds): axum::extract::State<FakeDs>, req: Request) -> Response {
    let path = req.uri().path();
    let is_shape = path.starts_with("/shape/");
    match *req.method() {
        Method::PUT | Method::POST | Method::DELETE => StatusCode::OK.into_response(),
        Method::HEAD => ([("stream-next-offset", "tip")], "").into_response(),
        Method::GET if is_shape => match ds.read_mode.load(Ordering::SeqCst) {
            1 => (StatusCode::NO_CONTENT, [("stream-closed", "true"), ("stream-next-offset", "tip")]).into_response(),
            2 => StatusCode::NOT_FOUND.into_response(),
            3 => StatusCode::GONE.into_response(),
            4 => StatusCode::SERVICE_UNAVAILABLE.into_response(),
            _ => ([("stream-next-offset", "tip"), ("stream-up-to-date", "1")], "[]").into_response(),
        },
        Method::GET => ([("stream-next-offset", "tip"), ("stream-up-to-date", "1")], "[]").into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

async fn app_with_ds() -> (Router, FakeDs) {
    let ds = FakeDs::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(ds_handler).with_state(ds.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let engine = Engine::new(DsClient::new(format!("http://{address}")));
    let schema: Schema = serde_json::from_value(serde_json::json!({
        "tables": { "items": { "columns": { "id": { "type": "text" } }, "primaryKey": "id" } }
    }))
    .unwrap();
    engine.define_schema(&schema).await.unwrap();
    (router(engine), ds)
}

async fn snapshot(app: &Router) -> (String, String) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/v1/shape?table=items&offset=-1").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    (
        response.headers().get("electric-handle").unwrap().to_str().unwrap().to_owned(),
        response.headers().get("electric-offset").unwrap().to_str().unwrap().to_owned(),
    )
}

async fn body(response: Response) -> String {
    String::from_utf8(axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap().to_vec()).unwrap()
}

#[tokio::test]
async fn closed_stream_returns_must_refetch_without_stale_handle() {
    let (app, ds) = app_with_ds().await;
    let (handle, offset) = snapshot(&app).await;
    ds.read_mode.store(1, Ordering::SeqCst);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/shape?table=items&handle={handle}&offset={offset}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(response.headers().get("electric-offset").unwrap(), "-1");
    assert!(response.headers().get("electric-handle").is_none());
    assert!(body(response).await.contains("must-refetch"));
}

#[tokio::test]
async fn deleted_stream_positioned_read_returns_must_refetch() {
    let (app, ds) = app_with_ds().await;
    let (handle, offset) = snapshot(&app).await;
    ds.read_mode.store(2, Ordering::SeqCst);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/shape?table=items&handle={handle}&offset={offset}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(response.headers().get("electric-offset").unwrap(), "-1");
    assert!(response.headers().get("electric-handle").is_none());
    assert!(body(response).await.contains("must-refetch"));
}

#[tokio::test]
async fn gone_stream_live_read_returns_must_refetch() {
    let (app, ds) = app_with_ds().await;
    let (handle, offset) = snapshot(&app).await;
    ds.read_mode.store(3, Ordering::SeqCst);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/shape?table=items&handle={handle}&offset={offset}&live=true"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(response.headers().get("electric-offset").unwrap(), "-1");
    assert!(response.headers().get("electric-handle").is_none());
    assert!(body(response).await.contains("must-refetch"));
}

#[tokio::test]
async fn transient_stream_read_remains_internal_server_error() {
    let (app, ds) = app_with_ds().await;
    let (handle, offset) = snapshot(&app).await;
    ds.read_mode.store(4, Ordering::SeqCst);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/shape?table=items&handle={handle}&offset={offset}&live=true"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body(response).await.contains("503"));
}
