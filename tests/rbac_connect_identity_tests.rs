//! persea#253: the RBAC Connect check in `rbac_connect_allowed` used to
//! feed `display_name()` into `get_user_by_email`. For LDAP/SSO-style
//! users the identity's `name` holds a human name (`cn`), not the email,
//! so the lookup returned no DB user and every Connect grant was
//! silently denied. These tests pin the contract: the lookup keys on
//! the user's EMAIL, even when the display name is set to something
//! that is definitely not the email.

use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{middleware, Extension, Router};
use persea::api::address_book::quick_connect;
use persea::api::OidcEnabled;
use persea::auth::{optional_auth, TrustedProxies};
use persea::db::{self, Db};
use persea::rbac;
use persea::session::SessionManager;
use std::net::SocketAddr;
use tower::ServiceExt;

type AppState = std::sync::Arc<SessionManager>;
type VaultState = std::sync::Arc<persea::api::VaultBackends>;

fn test_db() -> Db {
    db::init_db(std::path::Path::new(":memory:")).unwrap()
}

fn test_vault_state() -> VaultState {
    let cell: persea::api::VaultCell = std::sync::Arc::new(tokio::sync::RwLock::new(None));
    std::sync::Arc::new(persea::api::VaultBackends {
        default: cell.clone(),
        shared: cell.clone(),
        local: cell,
    })
}

fn build_quick_connect_router(db: Db, vault: VaultState) -> Router {
    let manager: AppState = std::sync::Arc::new(SessionManager::new_with_db(
        persea::config::Config::default(),
        None,
        db.clone(),
    ));
    Router::new()
        .route("/api/connect", get(quick_connect))
        .with_state(manager)
        .layer(middleware::from_fn(optional_auth))
        .layer(Extension(vault))
        .layer(Extension(OidcEnabled(false)))
        .layer(Extension(db))
}

fn session_req(method: &str, uri: &str, session: &str) -> Request<axum::body::Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", format!("persea_session={}", session))
        .body(axum::body::Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo("127.0.0.1:0".parse::<SocketAddr>().unwrap()));
    req
}

/// A user whose stored display name is NOT their email.
fn user_with_display_name(db: &Db, email: &str, name: &str, role: &str) -> String {
    let user = db::upsert_user(db, email, name, None, role, &[]).unwrap();
    db::create_auth_session(db, user.id, 3600).unwrap()
}

const ENTRY_PATH: &str = "/api/connect?scope=shared&folder=Clients&entry=web1";

fn fixture_entry(db: &Db) {
    db::create_ab_folder(db, "shared", "Clients", "", "", false).unwrap();
    let folder = db::get_ab_folder(db, "shared", "Clients").unwrap();
    db::create_ab_entry(
        db,
        folder.id,
        "web1",
        "Web 1",
        "ssh",
        "10.0.0.1",
        Some(22),
        "root",
        "{}",
        "",
    )
    .unwrap();
}

#[tokio::test]
async fn connect_grant_allows_user_with_display_name() {
    // The regression: an LDAP/SSO-style identity whose name field holds
    // "Alice Example" while the email is alice@example.com; the grant
    // must resolve through the email.
    let db = test_db();
    fixture_entry(&db);
    let session = user_with_display_name(&db, "alice@example.com", "Alice Example", "operator");
    let user = db::get_user_by_email(&db, "alice@example.com").unwrap();
    assert_eq!(user.name, "Alice Example");

    rbac::grant_connection_permission(
        &db,
        &format!("u:{}", user.id),
        "shared/Clients/web1",
        rbac::ObjectPermission::Connect,
    )
    .unwrap();

    let response = build_quick_connect_router(db, test_vault_state())
        .oneshot(session_req("GET", ENTRY_PATH, &session))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a display name must not break an email-keyed Connect grant"
    );
}

#[tokio::test]
async fn connect_denied_without_grant_even_with_display_name() {
    let db = test_db();
    fixture_entry(&db);
    let session = user_with_display_name(&db, "bob@example.com", "Bobby Tables", "operator");
    assert_ne!(
        db::get_user_by_email(&db, "bob@example.com").unwrap().name,
        "bob@example.com"
    );

    let response = build_quick_connect_router(db, test_vault_state())
        .oneshot(session_req("GET", ENTRY_PATH, &session))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "no grant: denied, regardless of the display name"
    );
}

#[tokio::test]
async fn connect_without_grant_still_denied() {
    // Regression guard: the unnamed-operator deny behavior must survive
    // the lookup fix.
    let db = test_db();
    fixture_entry(&db);
    let session = user_with_display_name(&db, "carl@example.com", "carl@example.com", "operator");

    let response = build_quick_connect_router(db, test_vault_state())
        .oneshot(session_req("GET", ENTRY_PATH, &session))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
