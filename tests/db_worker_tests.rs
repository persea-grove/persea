//! Regression test for the dedicated database worker thread.
//!
//! Separate integration binary because `set_active_pool` installs a
//! process-global pool exactly once.

use persea::db;
use persea::db_pool::DbPool;

#[tokio::test]
async fn worker_survives_a_panicking_query() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite pool");
    db::set_active_pool(DbPool::SQLite(pool))
        .ok()
        .expect("pool installs once");

    let panicked = db::pool_call::<_, _, i64, rusqlite::Error>(|_pool| async move {
        panic!("intentional worker panic");
    });
    let err = panicked.expect_err("a panic must surface as a query error");
    assert!(
        err.to_string().contains("panicked"),
        "unexpected error: {err}"
    );

    // The same worker must still serve queries afterwards.
    let value = db::pool_call::<_, _, i64, rusqlite::Error>(|_pool| async move { Ok(42) })
        .expect("worker still alive after a panicking query");
    assert_eq!(value, 42);
}
