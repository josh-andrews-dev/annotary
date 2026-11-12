//! A count of the SQL statements every [`Store`](crate::Store) connection in this process has
//! run (`stmt-count` feature, tests only). The size report reads it before and after one request
//! to say how many statements composing an answer took.

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::trace::{TraceEvent, TraceEventCodes};

static STATEMENTS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn install(conn: &rusqlite::Connection) {
    conn.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, Some(on_trace));
}

fn on_trace(event: TraceEvent<'_>) {
    // SQLite also reports each trigger a statement fires, as text starting with `--`.
    if let TraceEvent::Stmt(_, sql) = event
        && !sql.starts_with("--")
    {
        STATEMENTS.fetch_add(1, Ordering::SeqCst);
    }
}

/// Statements run so far, by every connection opened through `Store::open`.
pub fn statements() -> u64 {
    STATEMENTS.load(Ordering::SeqCst)
}
