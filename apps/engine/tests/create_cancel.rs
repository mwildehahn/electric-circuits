//! A create whose future is dropped mid-flight (the HTTP client disconnected) must give back
//! everything it registered, so the next identical create behaves as if the first never happened.
//!
//! The drop point here is the create's first await after registration — the stream PUT — held open
//! by a durable-streams stub that parks every PUT on demand. No Postgres: in library mode the
//! backfill is empty, so a plain create that is allowed to finish succeeds.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use electric_circuits_engine::ds::DsClient;
use electric_circuits_engine::engine::Engine;
use electric_circuits_engine::predicate::PredicateJson;
use electric_circuits_engine::schema::Schema;
use electric_circuits_engine::table_ref::TableRef;

/// Table refs in tests: bare names are the `public.<name>` sugar.
fn tref(name: &str) -> TableRef {
    TableRef::parse(name).unwrap()
}

#[derive(Clone, Default)]
struct FakeDs {
    /// While set, every stream PUT parks — the create under test blocks there instead of racing.
    park_puts: Arc<AtomicBool>,
    deleted: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn ds_handler(State(ds): State<FakeDs>, request: Request) -> Response {
    match *request.method() {
        Method::PUT => {
            while ds.park_puts.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            StatusCode::OK.into_response()
        }
        Method::DELETE => {
            ds.deleted.lock().unwrap().push(request.uri().path().to_string());
            StatusCode::OK.into_response()
        }
        Method::POST => StatusCode::OK.into_response(),
        Method::GET => ([("stream-next-offset", "tip"), ("stream-up-to-date", "1")], "[]").into_response(),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

async fn engine_on_fake_ds() -> (Engine, FakeDs) {
    let ds = FakeDs::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(ds_handler).with_state(ds.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let engine = Engine::new(DsClient::new(format!("http://{address}")));
    let schema: Schema = serde_json::from_value(serde_json::json!({
        "tables": {
            "parent": { "columns": { "id": {"type":"int"}, "active": {"type":"bool"} }, "primaryKey": "id" },
            "child": { "columns": { "id": {"type":"int"}, "parent_id": {"type":"int"} }, "primaryKey": "id" }
        }
    }))
    .unwrap();
    engine.define_schema(&schema).await.unwrap();
    (engine, ds)
}

/// Park the create on the stream PUT, drop its future, then let the PUT through and wait for the
/// detached rollback to retire the shape record.
async fn cancel_create(engine: &Engine, ds: &FakeDs, where_: &PredicateJson) {
    ds.park_puts.store(true, Ordering::SeqCst);
    let cancelled = tokio::time::timeout(
        Duration::from_millis(300),
        engine.create_shape(&tref("child"), Some(where_.clone()), None, false, true),
    )
    .await;
    assert!(cancelled.is_err(), "the create must still be parked when its future is dropped");
    ds.park_puts.store(false, Ordering::SeqCst);
    // The rollback is detached, so poll until BOTH halves have landed: the engine-state entries and
    // the stream itself.
    let deleted = || ds.deleted.lock().unwrap().iter().any(|path| path.ends_with("shape/s1"));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while engine.get_shape("s1").await.is_some() || !deleted() {
        assert!(std::time::Instant::now() < deadline, "the cancelled create was never rolled back");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(engine.graph().await.shapes.is_empty(), "no shape may survive the rollback");
}

/// Plain shape: the second create must make a NEW shape (its own id and stream) rather than join
/// the abandoned one and fail with "creator died before completing".
#[tokio::test(flavor = "multi_thread")]
async fn cancelled_plain_create_lets_the_same_shape_be_created_again() {
    let (engine, ds) = engine_on_fake_ds().await;
    let where_: PredicateJson =
        serde_json::from_value(serde_json::json!({"col":"parent_id","op":"eq","value":1})).unwrap();

    cancel_create(&engine, &ds, &where_).await;

    let rec = engine
        .create_shape(&tref("child"), Some(where_), None, false, true)
        .await
        .expect("the identical create must succeed once the cancelled one is rolled back");
    assert_ne!(rec.id, "s1", "the recreate must not reuse the abandoned shape");
    assert_eq!(engine.graph().await.shapes.len(), 1);
}

/// Subquery shape: the same rollback, on the path whose registration also spans the cross-table
/// registry. Library mode has no Postgres to seed the inner set from, so the recreate cannot
/// finish — but it must fail on its OWN missing Postgres, not on the abandoned create's share
/// entry, and it must never see a "subquery create conflict" from leftover registry state.
#[tokio::test(flavor = "multi_thread")]
async fn cancelled_subquery_create_does_not_poison_the_next_one() {
    let (engine, ds) = engine_on_fake_ds().await;
    let where_: PredicateJson = serde_json::from_value(serde_json::json!({
        "col": "parent_id",
        "in": { "table": "parent", "project": "id", "where": {"col":"active","op":"eq","value":true} }
    }))
    .unwrap();

    cancel_create(&engine, &ds, &where_).await;

    let err = engine
        .create_shape(&tref("child"), Some(where_), None, false, true)
        .await
        .expect_err("library mode cannot seed a subquery shape")
        .to_string();
    assert!(err.contains("postgres"), "unexpected failure: {err}");
    assert!(engine.subquery_stats().await.is_empty(), "no registry node may survive");
}
