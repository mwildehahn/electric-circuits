//! Gateway credentials authenticate at the real Shape HTTP boundary, before table lookup.
//! This binary owns its process-global credentials and does not require a database or streams.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use electric_circuits_engine::{config, ds::DsClient, engine::Engine, http::router};
use tower::ServiceExt;

#[tokio::test]
async fn shape_accepts_gateway_bearer_and_rejects_other_credentials() {
    config::set_globals("shape-auth-test", "shape-auth-test", Some("gateway-secret"), Some("controller-secret"));
    let engine = Engine::new_for_in_process_test(DsClient::new_for_in_process_test("http://127.0.0.1:1"));
    let app = router(engine);
    for (authorization, query, expected) in [
        (Some("Bearer gateway-secret"), "", StatusCode::BAD_REQUEST),
        (None, "", StatusCode::UNAUTHORIZED),
        (Some("Bearer wrong-secret"), "", StatusCode::UNAUTHORIZED),
        (Some("Bearer controller-secret"), "", StatusCode::UNAUTHORIZED),
        (Some("Basic gateway-secret"), "", StatusCode::UNAUTHORIZED),
        (None, "&secret=gateway-secret", StatusCode::BAD_REQUEST),
        (None, "&api_secret=gateway-secret", StatusCode::BAD_REQUEST),
    ] {
        let mut request = Request::get(format!("/v1/shape?table=not_published&offset=-1{query}"));
        if let Some(authorization) = authorization {
            request = request.header("authorization", authorization);
        }
        let response = app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), expected, "authorization={authorization:?}, query={query}");
    }
}
