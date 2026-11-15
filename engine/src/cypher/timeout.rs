//! `og.statement_timeout` — how long one Cypher statement may run.
//!
//! A pattern over virtual nodes is a query against a source system, and a
//! cartesian product of two remote tables is one missing relationship away:
//! `MATCH (a:Customer), (b:Shipment) RETURN count(*)`. Nothing stopped one.
//! PostgreSQL's own `statement_timeout` is off by default and no caller set it,
//! so such a statement ran until the source finished — hours — holding a
//! backend and a source connection the whole time.
//!
//! The limit is PostgreSQL's statement timer, armed here when a Cypher
//! statement starts. When it fires the backend cancels exactly as it does for
//! `statement_timeout`: the executor stops at its next interrupt check,
//! postgres_fdw sends the source a cancel request for the remote half, and the
//! caller receives SQLSTATE 57014, "canceling statement due to statement
//! timeout". Nothing here catches that error, so nothing can swallow it.
//!
//! The setting is a placeholder (`og.` settings are not GUCs this extension
//! defines, see `og.graph`, `og.actor`), so it needs no preloading and any role
//! may set it for its own session or transaction:
//!
//! ```sql
//! SET og.statement_timeout = '30s';      -- or 30000 (milliseconds), '2min', 0 = no limit
//! ALTER DATABASE og SET og.statement_timeout = '300s';
//! ```
//!
//! Unset means [`DEFAULT_MS`]. A stricter `statement_timeout` already running
//! for the statement is left alone: the earlier deadline wins.

use pgrx::pg_sys;
use pgrx::prelude::*;

/// Two minutes: long enough for any honest question over a source table, short
/// enough that an agent's turn gets its error while it can still act on it.
pub const DEFAULT_MS: i64 = 120_000;

/// `TimeoutId::STATEMENT_TIMEOUT` in `utils/timeout.h`. Fourth in the enum
/// since PostgreSQL 9.3, and the bindings do not carry that header.
const STATEMENT_TIMEOUT: core::ffi::c_int = 3;

extern "C-unwind" {
    fn enable_timeout_after(id: core::ffi::c_int, delay_ms: core::ffi::c_int);
    fn get_timeout_active(id: core::ffi::c_int) -> bool;
    fn get_timeout_finish_time(id: core::ffi::c_int) -> pg_sys::TimestampTz;
}

/// `og.statement_timeout` as milliseconds; 0 means no limit.
pub fn configured_ms() -> i64 {
    let raw = crate::spiu::one::<String>("SELECT current_setting('og.statement_timeout', true)", &[])
        .ok()
        .flatten()
        .unwrap_or_default();
    match parse_ms(&raw) {
        Ok(Some(ms)) => ms,
        Ok(None) => DEFAULT_MS,
        Err(e) => error!("og.statement_timeout: {e}"),
    }
}

/// Start the clock for the statement that is running now.
///
/// Called where a Cypher statement begins. Calling it again within the same
/// top-level statement — a PL/pgSQL loop over `og_cypher()`, an action running
/// several queries — does not extend the deadline: the timer is only ever
/// moved earlier. PostgreSQL disarms it when the statement ends.
pub fn arm() {
    let ms = configured_ms();
    if ms <= 0 {
        return;
    }
    let ms = ms.min(i32::MAX as i64);
    unsafe {
        pg_sys::ffi::pg_guard_ffi_boundary(|| {
            if get_timeout_active(STATEMENT_TIMEOUT) {
                let mine = pg_sys::GetCurrentTimestamp() + ms * 1000;
                if get_timeout_finish_time(STATEMENT_TIMEOUT) <= mine {
                    return;
                }
            }
            enable_timeout_after(STATEMENT_TIMEOUT, ms as core::ffi::c_int);
        })
    }
}

/// The limit a statement started now would get, in milliseconds (0 = none).
/// For clients that want to say what the limit was when it is hit — the Bolt
/// gateway reads it once per session.
#[pg_extern(stable)]
fn og_statement_timeout() -> i64 {
    configured_ms()
}

/// `''` → None (use the default). A bare number is milliseconds, as for
/// PostgreSQL's own `statement_timeout`; otherwise a number and one of the
/// units PostgreSQL accepts for time settings.
pub fn parse_ms(raw: &str) -> Result<Option<i64>, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let value: f64 = num
        .parse()
        .map_err(|_| format!("'{raw}' is not a duration — use e.g. 120000, '120s' or '2min'"))?;
    let scale = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "ms" => 1.0,
        "s" => 1_000.0,
        "min" => 60_000.0,
        "h" => 3_600_000.0,
        "d" => 86_400_000.0,
        other => return Err(format!("unknown unit '{other}' in '{raw}' — use ms, s, min, h or d")),
    };
    Ok(Some((value * scale).round() as i64))
}

#[cfg(test)]
mod tests {
    use super::parse_ms;

    #[test]
    fn units() {
        assert_eq!(parse_ms(""), Ok(None));
        assert_eq!(parse_ms(" 0 "), Ok(Some(0)));
        assert_eq!(parse_ms("1500"), Ok(Some(1500)));
        assert_eq!(parse_ms("2s"), Ok(Some(2000)));
        assert_eq!(parse_ms("2 min"), Ok(Some(120_000)));
        assert_eq!(parse_ms("1.5s"), Ok(Some(1500)));
        assert_eq!(parse_ms("250ms"), Ok(Some(250)));
        assert!(parse_ms("soon").is_err());
        assert!(parse_ms("5 parsecs").is_err());
    }
}
