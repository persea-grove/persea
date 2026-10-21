//! Pairing insert routing through the db worker runtime (persea#319).
//!
//! `insert_pairing` used to await the SQLx pool query directly on the axum
//! runtime, which risks the sqlx cross-runtime acquire race (lost readiness
//! wakeup, observed as a full 30s `acquire_timeout` stall). The insert now
//! goes through `db::insert_active_pairing`, which dispatches on the
//! dedicated persea-db-worker thread and reports `Ok(false)` when no pool
//! store is active so the caller falls back to the legacy SQLite path.
//!
//! The worker routing itself has no deterministic seam (a direct await also
//! succeeds on a fast machine, and `pool_call` is synchronous from the
//! caller's side), so the assertions pin the observable contract instead:
//! the fallback sentinel before a pool is installed, a successful insert
//! via the pool store once installed, and the row landing in the pool
//! store's database.
//!
//! The #322 tests extend the same contract to the remaining sibling
//! helpers: lookup, bind, claim, named-token revoke and user-identity are
//! routed through the matching `db::*_active_*` worker wrappers, with the
//! `None` sentinel (no pool store active) standing in for the legacy
//! SQLite fallback.
//!
//! This lives in its own test binary because the pool store is a
//! process-global `OnceLock`: installing it here must not flip the
//! pairing flow tests (which exercise the SQLite fallback) onto the pool
//! path, and the warm-up test binary already consumes its own install.

use persea::db;
use persea::db_pool::DbPool;

/// The pool store is a process-global install: serialize both tests' setup
/// so the sentinel-before-install asserts never race the install itself and
/// only one install (and one `persea-db-worker` thread) happens.
static POOL_SETUP: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_insert_uses_pool_store_when_active_and_reports_fallback_when_not() {
    let _guard = POOL_SETUP.lock().unwrap_or_else(|p| p.into_inner());
    // No pool store installed yet: the wrapper must hand back the fallback
    // sentinel instead of erroring or touching any database.
    if !db::pool_active() {
        let sentinel = db::insert_active_pairing(
            "pairing-route-sentinel",
            "sentinel-box",
            "2030-01-01 00:00:00",
        );
        assert!(
            matches!(sentinel, Ok(false)),
            "expected the no-pool fallback sentinel, got {sentinel:?}"
        );

        // Mirror main.rs: connect + run migrations on the test's main runtime,
        // THEN install the pool as the active store (spawns the db worker).
        let dir = std::env::temp_dir().join(format!("persea-319-pairing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sqlite_path = dir.join("pairing.db");
        let pool = DbPool::connect(&format!("sqlite://{}?mode=rwc", sqlite_path.display()))
            .await
            .unwrap();
        pool.run_migrations().await.unwrap();
        db::set_active_pool(pool).unwrap();

        // Pool store active: the insert must succeed through the worker.
        let inserted =
            db::insert_active_pairing("pairing-route-hash", "route-box", "2030-01-01 00:00:00");
        assert!(
            matches!(inserted, Ok(true)),
            "pool-store insert failed: {inserted:?}"
        );

        // The row landed in the pool store's database (read back with rusqlite
        // through the same file the SQLx SQLite pool owns).
        let conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        let (hostname, expires_at): (String, String) = conn
            .query_row(
                "SELECT hostname, expires_at FROM desktop_pairings WHERE code_hash = ?1",
                rusqlite::params!["pairing-route-hash"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(hostname, "route-box");
        assert_eq!(expires_at, "2030-01-01 00:00:00");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_lookup_bind_revoke_route_through_worker_and_fallback_when_not_active() {
    let _guard = POOL_SETUP.lock().unwrap_or_else(|p| p.into_inner());
    // No pool store installed yet: the wrappers must report `None` (the
    // legacy SQLite fallback sentinel) instead of erroring or touching any
    // database.
    if !db::pool_active() {
        assert!(db::lookup_active_pairing("lookup-sentinel-hash").is_none());
        assert!(db::bind_active_pairing("bind-sentinel-hash", 1, "2030-01-01 00:00:00").is_none());
        assert!(db::claim_active_pairing("claim-sentinel-hash", "2030-01-01 00:00:00").is_none());
        assert!(db::revoke_tokens_named_active(1, "sentinel-name").is_none());
        assert!(db::user_identity_by_id_active(1).is_none());

        // Mirror main.rs: connect + run migrations on the test's main
        // runtime, THEN install the pool as the active store (spawns the db
        // worker).
        let dir = std::env::temp_dir().join(format!("persea-322-pairing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sqlite_path = dir.join("pairing.db");
        let pool = DbPool::connect(&format!("sqlite://{}?mode=rwc", sqlite_path.display()))
            .await
            .unwrap();
        pool.run_migrations().await.unwrap();
        db::set_active_pool(pool).unwrap();

        // The pairing row is readable back through the worker-routed
        // lookup, and the pending fields round-trip.
        let inserted =
            db::insert_active_pairing("lookup-route-hash", "lookup-box", "2030-01-01 00:00:00");
        assert!(
            matches!(inserted, Ok(true)),
            "pool-store insert failed: {inserted:?}"
        );
        let row = db::lookup_active_pairing("lookup-route-hash")
            .expect("pool store is active")
            .expect("worker-routed lookup failed")
            .expect("pairing row not found in the pool store");
        assert!(row.user_id.is_none());
        assert_eq!(row.hostname, "lookup-box");
        assert_eq!(row.expires_at, "2030-01-01 00:00:00");
        assert!(row.consumed_at.is_none());

        // A pending row can be bound by a user that exists, and the second
        // bind loses the single-use race (Ok(false), no error).
        let conn = rusqlite::Connection::open(&sqlite_path).unwrap();
        conn.execute(
            "INSERT INTO users (email, role) VALUES ('pairing-test@example.com', 'operator')",
            [],
        )
        .unwrap();
        let user_id: i64 = conn.last_insert_rowid();
        drop(conn);
        let bound = db::bind_active_pairing("lookup-route-hash", user_id, "2029-01-01 00:00:00")
            .expect("pool store is active")
            .expect("worker-routed bind failed");
        assert!(bound, "expected the first bind to win");
        let rebound = db::bind_active_pairing("lookup-route-hash", user_id, "2029-01-01 00:00:00")
            .expect("pool store is active")
            .expect("worker-routed rebind gave an error instead of Ok(false)");
        assert!(!rebound, "expected the second bind to lose the race");

        // Once bound, the claim hands the token plaintext out exactly once.
        let claimed = db::claim_active_pairing("lookup-route-hash", "2029-12-01 00:00:00")
            .expect("pool store is active")
            .expect("worker-routed claim failed");
        assert!(claimed, "expected the first claim to win");
        let reclaimed = db::claim_active_pairing("lookup-route-hash", "2029-12-01 00:00:00")
            .expect("pool store is active")
            .expect("worker-routed reclaim gave an error instead of Ok(false)");
        assert!(!reclaimed, "expected the second claim to lose the race");

        // Revoking another user's tokens is a plain count (nothing to lose).
        let revoked = db::revoke_tokens_named_active(user_id, "no-such-token-name")
            .expect("pool store is active")
            .expect("worker-routed revoke failed");
        assert_eq!(revoked, 0);

        // Unknown user id surfaces as Ok(None), not an error.
        let identity = db::user_identity_by_id_active(4_242_424).expect("pool store is active");
        assert!(
            matches!(identity, Ok(None)),
            "unexpected identity: {identity:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    } else {
        // The sibling test in this binary installed the pool store already
        // (process-global `OnceLock`, only one install wins). The pool
        // store is active: every wrapper answers through the worker, and a
        // not-found lookup stays `Ok(None)` instead of triggering the
        // legacy SQLite fallback.
        assert!(matches!(
            db::lookup_active_pairing("shared-pool-unknown-hash").unwrap(),
            Ok(None)
        ));
        assert!(matches!(
            db::revoke_tokens_named_active(1, "shared-pool-unknown-name").unwrap(),
            Ok(0)
        ));
        assert!(matches!(
            db::user_identity_by_id_active(4_242_424).unwrap(),
            Ok(None)
        ));
    }
}
