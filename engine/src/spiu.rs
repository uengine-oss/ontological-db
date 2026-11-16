//! SPI helpers.
//!
//! `Spi::get_one_with_args` raises `InvalidPosition` when a query returns no
//! rows, which conflates "nothing matched" with "something broke". Every lookup
//! in this extension needs to tell those apart — a missing type is a normal,
//! reportable condition (spec 008 FR-008), not an internal error.

use pgrx::datum::DatumWithOid;
use pgrx::prelude::*;
use pgrx::spi;
use std::cell::RefCell;

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

/// Read-write two-value lookup, for a data-modifying CTE that reports back.
pub fn two_mut<'a, A: FromDatum + IntoDatum, B: FromDatum + IntoDatum>(
    sql: &str,
    args: &[DatumWithOid<'a>],
) -> Result<(Option<A>, Option<B>), spi::Error> {
    Spi::connect_mut(|client| {
        let mut table = client.update(sql, Some(1), args)?;
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

// --------------------------------------------------------------------------
// Engine function names — the edition seam (P1 K6, `DESIGN.md` 5.3 (1))
// --------------------------------------------------------------------------
//
// Two editions can be installed in one database: `ontological` (v1, no
// `schema` in its control file, so its functions land in the schema it was
// created in, usually `public`) and `ontological_age` (v2, `schema =
// 'oga_catalog'`). Their SQL functions have the same names. A statement the
// engine runs or generates that calls one of them unqualified resolves it
// through `search_path` — and would reach the other edition's function. So
// every engine function name the editions' shared code writes into SQL comes
// from `fnq`.
//
// v1 never calls `set_edition`, and then `fnq` returns the name as it is: v1's
// SQL stays byte for byte what it was (the generated-SQL mold checks it). An
// edition that does call it (its `_PG_init`, K7) gets every name qualified
// with the schema its extension was installed in.

thread_local! {
    /// The extension this backend's edition is, if it said (`set_edition`).
    static EDITION: RefCell<Option<&'static str>> = const { RefCell::new(None) };
    /// That extension's schema, found once per backend (`relocatable = false`,
    /// so it cannot move), already quoted.
    static EDITION_SCHEMA: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Say which extension this library is running as: `set_edition("ontological_age")`.
/// Called once, from the edition's `_PG_init`; v1 does not call it.
pub fn set_edition(ext_name: &'static str) {
    EDITION.with(|e| *e.borrow_mut() = Some(ext_name));
    EDITION_SCHEMA.with(|s| *s.borrow_mut() = None);
}

/// The extension name `set_edition` was given, if it was called.
pub fn edition() -> Option<&'static str> {
    EDITION.with(|e| *e.borrow())
}

/// The name of an engine SQL function as the shared code is to write it into a
/// statement: as given when no edition was set (v1), `"<schema>".<name>` when
/// one was, `<schema>` being the schema the edition's extension is installed in.
pub fn fnq(name: &str) -> String {
    let Some(ext) = edition() else {
        return name.to_string();
    };
    let schema = EDITION_SCHEMA.with(|s| s.borrow().clone()).unwrap_or_else(|| {
        let found = ident(&edition_schema(ext));
        EDITION_SCHEMA.with(|s| *s.borrow_mut() = Some(found.clone()));
        found
    });
    format!("{schema}.{name}")
}

/// Where the extension `ext` is installed (`pg_extension.extnamespace`).
#[cfg(not(test))]
fn edition_schema(ext: &str) -> String {
    one::<String>(
        "SELECT n.nspname::text FROM pg_catalog.pg_extension e
           JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace
          WHERE e.extname = $1",
        &[ext.into()],
    )
    .unwrap_or_else(|e| error!("finding the schema of extension {ext} failed: {e}"))
    .unwrap_or_else(|| error!("extension {ext} is not installed in this database"))
}

/// The unit tests have no database: the schema lookup answers from what the
/// test says the catalog holds.
#[cfg(test)]
fn edition_schema(ext: &str) -> String {
    tests::CATALOG.with(|c| c.borrow().iter().find(|(e, _)| *e == ext).map(|(_, s)| s.to_string()))
        .unwrap_or_else(|| panic!("extension {ext} is not installed in this database"))
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// `pg_extension` as the test has it: (extension, schema).
        pub(super) static CATALOG: RefCell<Vec<(&'static str, &'static str)>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn without_an_edition_a_name_is_left_as_it_is() {
        // v1 never sets an edition; the lookup must not even be consulted.
        assert_eq!(edition(), None);
        assert_eq!(fnq("og_node_json"), "og_node_json");
        assert_eq!(fnq("og_cypher"), "og_cypher");
    }

    #[test]
    fn an_edition_qualifies_with_its_extensions_schema() {
        CATALOG.with(|c| c.borrow_mut().extend([("ontological", "public"), ("ontological_age", "oga_catalog")]));
        set_edition("ontological_age");
        assert_eq!(edition(), Some("ontological_age"));
        assert_eq!(fnq("og_cypher"), "\"oga_catalog\".og_cypher");
        // found once per backend: a later change to the catalog is not seen
        CATALOG.with(|c| c.borrow_mut().clear());
        assert_eq!(fnq("og_node_json"), "\"oga_catalog\".og_node_json");
    }

    #[test]
    fn a_schema_name_is_quoted() {
        CATALOG.with(|c| c.borrow_mut().push(("ext_b", "Odd \"Schema\"")));
        set_edition("ext_b");
        assert_eq!(fnq("og_vlp"), "\"Odd \"\"Schema\"\"\".og_vlp");
    }
}
