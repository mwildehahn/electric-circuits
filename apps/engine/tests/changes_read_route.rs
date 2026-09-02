//! Public positioned reads of the segmented change log (external-consumer contract).

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use electric_circuits_engine::ds::DsClient;
use electric_circuits_engine::engine::Engine;
use electric_circuits_engine::http::router;
use tower::ServiceExt;

#[derive(Clone, Default)]
struct FakeDs;

async fn durable_streams(State(_): State<FakeDs>, request: Request) -> Response {
    match *request.method() {
        Method::GET => (
            [
                ("stream-next-offset", "0000000000000000_0000000000000100"),
                ("stream-up-to-date", "true"),
            ],
            r#"[{"type":"public.items","key":"1","value":{"id":1},"old":null,"headers":{"operation":"insert","txid":"7","offset":null,"lsn":"0/10","seq":0,"last":true}}]"#,
        )
            .into_response(),
        Method::HEAD | Method::PUT | Method::POST | Method::DELETE => StatusCode::OK.into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

async fn engine() -> Engine {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, Router::new().fallback(durable_streams).with_state(FakeDs)).await;
    });
    Engine::new_for_in_process_test(DsClient::new_for_in_process_test(format!("http://{address}")))
}

#[tokio::test]
async fn returns_the_unmodified_change_log_page_and_stream_headers() {
    let response = router(engine().await)
        .oneshot(axum::http::Request::builder().uri("/changes/0?offset=-1").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["stream-next-offset"], "0000000000000000_0000000000000100");
    assert_eq!(response.headers()["stream-up-to-date"], "true");
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let page: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(page[0]["headers"]["last"], true);
    assert_eq!(page[0]["headers"]["lsn"], "0/10");
    assert_eq!(page[0]["headers"]["seq"], 0);
}
