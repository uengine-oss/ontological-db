//! SPI helpers.
//!
//! `Spi::get_one_with_args` raises `InvalidPosition` when a query returns no
//! rows, which conflates "nothing matched" with "something broke". Every lookup
//! in this extension needs to tell those apart — a missing type is a normal,
//! reportable condition (spec 008 FR-008), not an internal error.

use pgrx::datum::DatumWithOid;
use pgrx::prelude::*;
use pgrx::spi;

type R<T> = Result<Option<T>, spi::Error>;

/// Read-only single-value lookup. Returns `Ok(None)` for an empty result.
pub fn one<'a, T: FromDatum + IntoDatum>(sql: &str, args: &[DatumWithOid<'a>]) -> R<T> {
    Spi::connect(|client| {
        let mut table = client.select(sql, Some(1), args)?;
        match table.next() {
            Some(row) => row.get::<T>(1),
            None => Ok(None),
        }
    })
}

/// Read-only two-value lookup.
pub fn two<'a, A: FromDatum + IntoDatum, B: FromDatum + IntoDatum>(
    sql: &str,
    args: &[DatumWithOid<'a>],
) -> Result<(Option<A>, Option<B>), spi::Error> {
    Spi::connect(|client| {
        let mut table = client.select(sql, Some(1), args)?;
        match table.next() {
            Some(row) => Ok((row.get::<A>(1)?, row.get::<B>(2)?)),
            None => Ok((None, None)),
        }
    })
}

/// Read-write single-value lookup, for `INSERT ... RETURNING` and friends.
pub fn one_mut<'a, T: FromDatum + IntoDatum>(sql: &str, args: &[DatumWithOid<'a>]) -> R<T> {
    Spi::connect_mut(|client| {
        let mut table = client.update(sql, Some(1), args)?;
        match table.next() {
            Some(row) => row.get::<T>(1),
            None => Ok(None),
        }
    })
}

/// `security_invoker` makes a view read its base tables as the querying role
/// rather than as the view's owner. Without it, row-level security on the
/// storage tables is evaluated against whoever installed the extension, so
/// every policy a caller thinks is protecting them is answered by someone
/// else's identity. PostgreSQL gained the option in 15; on 13 and 14 there is
/// no equivalent, and a generated view is a hole straight through RLS.
#[cfg(any(feature = "pg13", feature = "pg14"))]
pub const VIEW_SECURITY: &str = "";
#[cfg(not(any(feature = "pg13", feature = "pg14")))]
pub const VIEW_SECURITY: &str = " WITH (security_invoker = true)";

/// Quote a string as a SQL literal.
///
/// Doubling `'` is enough while `standard_conforming_strings` is on, which it
/// has been by default since 9.1 — but it is a per-session setting a caller can
/// turn off, and then a backslash starts escaping again. So a string carrying a
/// backslash gets the explicit `E''` form, which means the same thing either
/// way. This mirrors what libpq's `PQescapeLiteral` does, for the same reason.
pub fn lit(s: &str) -> String {
    if s.contains('\\') {
        format!(" E'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

/// Quote a string as a SQL identifier.
///
/// Always quoted, never conditionally: deciding that a name "looks safe enough
/// to leave bare" is exactly the judgement that goes wrong. A NUL cannot appear
/// in a PostgreSQL identifier at all, so it is rejected rather than smuggled.
pub fn ident(s: &str) -> String {
    if s.contains('\0') {
        error!("identifier contains a NUL byte");
    }
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Quote a possibly schema-qualified name — `a.b` becomes `"a"."b"`.
///
/// Only the first dot splits: a schema name containing a dot is quoted whole on
/// the right-hand side, which is the reading that cannot invent a schema that
/// was not asked for.
pub fn qname(s: &str) -> String {
    match s.split_once('.') {
        Some((schema, rel)) => format!("{}.{}", ident(schema), ident(rel)),
        None => ident(s),
    }
}
