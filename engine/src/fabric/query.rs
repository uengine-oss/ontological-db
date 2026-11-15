//! `QUERY FABRIC` — one read-only SQL statement over the datasources' tables.
//!
//! ```text
//! QUERY FABRIC SELECT s.name, count(*) FROM erp.supplier s JOIN erp.purchase_order po USING (supplier_id) GROUP BY 1
//! QUERY FABRIC WITH {datasource: 'erp', max_rows: 1000, max_bytes: 67108864} SELECT … FROM supplier …
//! ```
//!
//! A datasource's tables are foreign tables in this database (`erp.supplier`),
//! and a SELECT over them is the fabric's own SQL surface: what a client that
//! thinks in tables — an SQL tool, a quality check — asks when a virtual node is
//! the wrong shape for the question. The statement is the fabric's, in the
//! fabric dialect, over the same connection as every other fabric statement
//! (`EXPLAIN FABRIC <cypher>` runs a query and reports on it; `QUERY FABRIC <sql>`
//! runs SQL over the sources). It is not a way into the engine: it never writes,
//! and reads datasource tables and nothing else. Writing to a source, and mixing
//! SQL into the graph, stay with virtual nodes (`CREATE VIRTUAL NODE … AS SELECT …`).
//!
//! The guards, each enough on its own for what it covers:
//!
//! 1. **The statement.** One `SELECT` or `WITH`, no `;`, balanced parentheses,
//!    no write keyword outside strings, quoted names and comments (42601).
//! 2. **The role.** It runs as `og_source_reader`: a role that cannot log in,
//!    inherits nothing, belongs to no role and holds `SELECT` on the
//!    datasources' foreign tables (and `USAGE` on their schemas) — granted as
//!    `CREATE` / `REFRESH DATASOURCE` bring tables in, and checked again before
//!    each query. The switch is a security-restricted user-id change, so
//!    `set_config('role', …)` and `session_authorization` are refused inside it
//!    and the statement cannot climb back to the caller's privileges. A caller
//!    may run it only if it could act as that role itself: a superuser, or a
//!    member (`GRANT og_source_reader TO …`).
//! 3. **What it may touch.** While it runs, every relation the executor is asked
//!    to read — directly, through a view, from a nested query — must be a
//!    datasource foreign table, and no `pg_*` or `og_*` function may run: the
//!    catalogs, the engine's own tables and the server's administrative
//!    functions are refused by name (42501) even where PUBLIC could reach them.
//! 4. **The transaction.** A subtransaction, read only, always rolled back:
//!    nothing the statement did — a setting, a temporary write — outlives it.
//!    The statement goes in as a subquery, where a data-modifying `WITH` cannot
//!    live, and the row cap is its `LIMIT`.
//! 5. **The limits.** The statement timer (`og.statement_timeout` — over Bolt, the
//!    driver's transaction timeout; cancelled here and at the source, 57014), at
//!    most `max_rows` rows (`truncated` says there were more) and `max_bytes` of
//!    values (54000).
//!
//! Values come back typed as a client reading PostgreSQL's text would type them:
//! booleans, integers and floats as JSON numbers (an `int8`/`numeric` of more
//! than 15 significant digits stays text, so it is never rounded), `json` /
//! `jsonb` as JSON, everything else — dates included — as PostgreSQL prints it.
//!
//! Every error carries its SQLSTATE in the message — `column "nope" does not
//! exist [SQLSTATE 42703, position 8]`, the position counted in the caller's SQL —
//! because a Bolt FAILURE has a Neo4j code and a message and nothing else, and a
//! client deciding between "fix the query" and "not allowed" needs the state.

use crate::spiu::{ident, lit, one};
use pgrx::pg_sys;
use pgrx::pg_sys::panic::CaughtError;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::{CStr, CString};

/// The role fabric queries run as.
pub const READER: &str = "og_source_reader";

const DEFAULT_MAX_ROWS: i64 = 1_000;
const CEILING_ROWS: i64 = 1_000_000;
/// The values of all rows together. A jsonb value cannot exceed 255 MB, and the
/// answer is copied a few times on its way out, so the ceiling is lower.
const DEFAULT_MAX_BYTES: i64 = 64 << 20;
const CEILING_BYTES: i64 = 128 << 20;
/// Rows fetched from the cursor at a time.
const BATCH: i64 = 200;

/// The statement goes in as a subquery: the row cap applies to it, and a
/// data-modifying `WITH` cannot live there.
const WRAP_HEAD: &str = "SELECT * FROM (\n";

/// Advisory-lock namespace for the reader's grants (`pg_advisory_xact_lock(ns, 0)`).
const LOCK_READER: i32 = 0x6f67_7301;

/// Keywords that write or reach outside a query. `INTO` because `SELECT … INTO`
/// creates a table. `REPLACE` is not here: `replace()` is a string function.
const WRITE_WORDS: &[&str] = &[
    "insert", "update", "delete", "merge", "truncate", "drop", "alter", "create", "grant", "revoke",
    "copy", "call", "exec", "execute", "load", "into",
];

// ------------------------------------------------------------------ errors

/// A SQLSTATE as its five characters (PostgreSQL packs six bits per character).
fn sqlstate(code: i32) -> String {
    let mut v = code;
    (0..5)
        .map(|_| {
            let c = ((v & 0x3F) as u8 + b'0') as char;
            v >>= 6;
            c
        })
        .collect()
}

fn tag(code: &str, position: Option<i64>) -> String {
    match position {
        Some(p) => format!("[SQLSTATE {code}, position {p}]"),
        None => format!("[SQLSTATE {code}]"),
    }
}

/// Raise an error whose message ends in its SQLSTATE.
fn raise(code: PgSqlErrorCode, message: &str) -> ! {
    raise_as(code, &sqlstate(code as i32), message, None, None, None)
}

/// [`raise`] with the SQLSTATE given as text — a code pgrx has no name for keeps
/// its own — and what the original error said besides its message.
fn raise_as(code: PgSqlErrorCode, state: &str, message: &str, position: Option<i64>, detail: Option<String>, hint: Option<String>) -> ! {
    let mut report = pg_sys::panic::ErrorReport::new(code, format!("{message} {}", tag(state, position)), "og_fabric_query");
    if let Some(d) = detail {
        report = report.set_detail(d);
    }
    if let Some(h) = hint {
        report = report.set_hint(h);
    }
    report.report(PgLogLevel::ERROR);
    unreachable!("an ERROR does not return")
}

fn refuse(message: &str) -> ! {
    raise(PgSqlErrorCode::ERRCODE_SYNTAX_ERROR, message)
}

// ------------------------------------------------------------- statement

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Lit,
    Name,
    Sym(char),
}

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c >= '\u{80}'
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$' || c >= '\u{80}'
}

fn is_tag_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c >= '\u{80}'
}

/// Lex `sql` the way PostgreSQL does for what matters here — strings (`'…'`,
/// `E'…'` with backslash escapes, `$tag$…$tag$`), quoted names, `--` and nested
/// `/* */` comments — and return the statement without a trailing `;`. Anything
/// but one SELECT or WITH statement is an `Err` with the reason.
pub fn check_statement(sql: &str) -> Result<String, String> {
    let mut text = sql.trim();
    while let Some(t) = text.strip_suffix(';') {
        text = t.trim_end();
    }
    if text.is_empty() {
        return Err("the statement is empty".into());
    }
    let cs: Vec<char> = text.chars().collect();
    let n = cs.len();
    let mut toks: Vec<Tok> = Vec::new();
    let mut depth = 0i32;
    let mut i = 0usize;

    // `i` at the opening quote; leaves it past the closing one.
    let quoted = |i: &mut usize, q: char, backslash: bool| -> Result<(), String> {
        *i += 1;
        loop {
            if *i >= n {
                return Err(if q == '"' { "a double-quoted name is not closed".into() } else { "a string literal is not closed".into() });
            }
            let c = cs[*i];
            if backslash && c == '\\' {
                *i += 2;
                continue;
            }
            if c == q {
                if *i + 1 < n && cs[*i + 1] == q {
                    *i += 2;
                    continue;
                }
                *i += 1;
                return Ok(());
            }
            *i += 1;
        }
    };

    while i < n {
        let c = cs[i];
        let d = if i + 1 < n { Some(cs[i + 1]) } else { None };
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '-' && d == Some('-') {
            while i < n && cs[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && d == Some('*') {
            let mut level = 1;
            i += 2;
            while i < n && level > 0 {
                if cs[i] == '/' && i + 1 < n && cs[i + 1] == '*' {
                    level += 1;
                    i += 2;
                } else if cs[i] == '*' && i + 1 < n && cs[i + 1] == '/' {
                    level -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if level > 0 {
                return Err("a /* … */ comment is not closed".into());
            }
            continue;
        }
        if c == '\'' {
            quoted(&mut i, '\'', false)?;
            toks.push(Tok::Lit);
            continue;
        }
        if c == '"' {
            quoted(&mut i, '"', false)?;
            toks.push(Tok::Name);
            continue;
        }
        if c == '$' {
            // $$ or $tag$ opens a dollar-quoted string; anything else ($1) is a symbol
            let mut j = i + 1;
            if j < n && (cs[j].is_ascii_alphabetic() || cs[j] == '_' || cs[j] >= '\u{80}') {
                while j < n && is_tag_char(cs[j]) {
                    j += 1;
                }
            }
            if j < n && cs[j] == '$' {
                let open: Vec<char> = cs[i..=j].to_vec();
                let mut k = j + 1;
                let mut end = None;
                while k + open.len() <= n {
                    if cs[k..k + open.len()] == open[..] {
                        end = Some(k);
                        break;
                    }
                    k += 1;
                }
                let Some(e) = end else { return Err("a $…$ string is not closed".into()) };
                i = e + open.len();
                toks.push(Tok::Lit);
                continue;
            }
            i += 1;
            toks.push(Tok::Sym('$'));
            continue;
        }
        if is_word_start(c) {
            let start = i;
            while i < n && is_word_char(cs[i]) {
                i += 1;
            }
            let w: String = cs[start..i].iter().collect();
            let next = if i < n { Some(cs[i]) } else { None };
            // E'…' takes backslash escapes; B'…', X'…', N'…', U&'…' / U&"…" are ordinary quotes after a prefix
            if next == Some('\'') && (w == "e" || w == "E") {
                quoted(&mut i, '\'', true)?;
                toks.push(Tok::Lit);
                continue;
            }
            if next == Some('\'') && matches!(w.as_str(), "b" | "B" | "x" | "X" | "n" | "N") {
                quoted(&mut i, '\'', false)?;
                toks.push(Tok::Lit);
                continue;
            }
            if (w == "u" || w == "U") && next == Some('&') && i + 1 < n && (cs[i + 1] == '\'' || cs[i + 1] == '"') {
                i += 1;
                let q = cs[i];
                quoted(&mut i, q, false)?;
                toks.push(if q == '"' { Tok::Name } else { Tok::Lit });
                continue;
            }
            toks.push(Tok::Word(w.to_lowercase()));
            continue;
        }
        if c == ';' {
            return Err("one statement at a time (found ';')".into());
        }
        if c == '(' {
            depth += 1;
        }
        if c == ')' {
            depth -= 1;
            if depth < 0 {
                return Err("parentheses do not match".into());
            }
        }
        toks.push(Tok::Sym(c));
        i += 1;
    }
    if depth != 0 {
        return Err("parentheses do not match".into());
    }
    match toks.iter().find(|t| **t != Tok::Sym('(')) {
        Some(Tok::Word(w)) if w == "select" || w == "with" => {}
        _ => return Err("only a SELECT or WITH statement can run here".into()),
    }
    if let Some(Tok::Word(w)) = toks.iter().find(|t| matches!(t, Tok::Word(w) if WRITE_WORDS.contains(&w.as_str()))) {
        return Err(format!("read only — {} cannot be used here (SELECT only)", w.to_uppercase()));
    }
    Ok(text.to_string())
}

// ------------------------------------------------------------------ values

/// 15 significant digits or fewer survive a double unchanged; longer ones stay text.
fn exact_number(v: &str) -> Value {
    let digits = v.trim_start_matches(['-', '+']).replacen('.', "", 1);
    if digits.trim_start_matches('0').len() <= 15 {
        if let Ok(i) = v.parse::<i64>() {
            return Value::from(i);
        }
        if let Ok(f) = v.parse::<f64>() {
            if let Some(num) = serde_json::Number::from_f64(f).filter(|_| f.is_finite()) {
                return Value::Number(num);
            }
        }
    }
    Value::String(v.to_string())
}

/// PostgreSQL prints a float in its shortest exact form; NaN and the infinities stay text.
fn finite(v: &str) -> Value {
    match v.parse::<f64>() {
        Ok(f) if f.is_finite() => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or_else(|| Value::String(v.into())),
        _ => Value::String(v.to_string()),
    }
}

fn typed(oid: u32, text: String) -> Value {
    match oid {
        16 => Value::Bool(text == "t"),                                                       // bool
        20 | 1700 => exact_number(&text),                                                     // int8, numeric
        21 | 23 | 26 => text.parse::<i64>().map(Value::from).unwrap_or(Value::String(text)),  // int2 int4 oid
        700 | 701 => finite(&text),                                                           // float4 float8
        114 | 3802 => serde_json::from_str(&text).unwrap_or(Value::String(text)),             // json jsonb
        _ => Value::String(text),
    }
}

// ------------------------------------------------------------------- reader

/// Make `og_source_reader` what it must be — a role that cannot log in, inherits
/// nothing, belongs to no role, has no settings — and grant it SELECT on every
/// datasource foreign table it cannot read yet (USAGE on their schemas). Nothing
/// else is granted. Cheap when nothing is missing (one catalog query); writes
/// only what is, under a lock so two sessions do not grant at once. Returns what
/// it changed.
pub fn sync_reader() -> String {
    if !needs_sync() {
        return "up to date".into();
    }
    crate::catalog::labeling::lock(LOCK_READER, 0);
    let mut changed = Vec::new();
    let exists = one::<bool>("SELECT true FROM pg_roles WHERE rolname = $1", &[READER.into()]).ok().flatten().unwrap_or(false);
    if !exists {
        Spi::run(&format!("CREATE ROLE {} NOLOGIN NOINHERIT", ident(READER)))
            .unwrap_or_else(|e| error!("could not create role {READER}: {e}"));
        changed.push("role created".to_string());
    }
    // An earlier application made this role itself, as one it logged in with.
    let loose = one::<bool>(
        "SELECT rolcanlogin OR rolsuper OR rolinherit OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls
           FROM pg_roles WHERE rolname = $1",
        &[READER.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if loose {
        Spi::run(&format!(
            "ALTER ROLE {} NOLOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD NULL",
            ident(READER)
        ))
        .unwrap_or_else(|e| error!("could not restrict role {READER}: {e}"));
        changed.push("role restricted".to_string());
    }
    let settings: Vec<Option<String>> = Spi::connect(|c| {
        c.select(
            "SELECT d.datname::text FROM pg_db_role_setting s LEFT JOIN pg_database d ON d.oid = s.setdatabase
              WHERE s.setrole = (SELECT oid FROM pg_roles WHERE rolname = $1)",
            None,
            &[READER.into()],
        )
        .map(|t| t.map(|r| r.get::<String>(1).ok().flatten()).collect())
        .unwrap_or_default()
    });
    for db in &settings {
        let scope = db.as_ref().map(|d| format!(" IN DATABASE {}", ident(d))).unwrap_or_default();
        Spi::run(&format!("ALTER ROLE {}{scope} RESET ALL", ident(READER)))
            .unwrap_or_else(|e| error!("could not reset the settings of {READER}: {e}"));
    }
    if !settings.is_empty() {
        changed.push("settings reset".to_string());
    }
    let memberships: Vec<String> = Spi::connect(|c| {
        c.select(
            "SELECT r.rolname::text FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.roleid
              WHERE m.member = (SELECT oid FROM pg_roles WHERE rolname = $1)",
            None,
            &[READER.into()],
        )
        .map(|t| t.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
        .unwrap_or_default()
    });
    for r in &memberships {
        Spi::run(&format!("REVOKE {} FROM {}", ident(r), ident(READER)))
            .unwrap_or_else(|e| error!("could not revoke {r} from {READER}: {e}"));
    }
    if !memberships.is_empty() {
        changed.push(format!("memberships revoked: {}", memberships.join(", ")));
    }
    let missing: Vec<(String, String, bool, bool)> = Spi::connect(|c| {
        c.select(MISSING_GRANTS, None, &[READER.into()])
            .map(|t| {
                t.filter_map(|r| {
                    Some((
                        r.get::<String>(1).ok()??,
                        r.get::<String>(2).ok()??,
                        r.get::<bool>(3).ok()??,
                        r.get::<bool>(4).ok()??,
                    ))
                })
                .collect()
            })
            .unwrap_or_default()
    });
    let mut schemas: Vec<&str> = missing.iter().filter(|m| !m.2).map(|m| m.0.as_str()).collect();
    schemas.dedup();
    for s in &schemas {
        Spi::run(&format!("GRANT USAGE ON SCHEMA {} TO {}", ident(s), ident(READER)))
            .unwrap_or_else(|e| error!("could not grant USAGE on schema {s} to {READER}: {e}"));
    }
    let tables: Vec<String> = missing.iter().filter(|m| !m.3).map(|m| format!("{}.{}", ident(&m.0), ident(&m.1))).collect();
    for t in &tables {
        Spi::run(&format!("GRANT SELECT ON {t} TO {}", ident(READER)))
            .unwrap_or_else(|e| error!("could not grant SELECT on {t} to {READER}: {e}"));
    }
    if !tables.is_empty() {
        changed.push(format!("{} tables granted", tables.len()));
    }
    if changed.is_empty() {
        "up to date".into()
    } else {
        changed.join(", ")
    }
}

/// Datasource foreign tables the reader cannot read yet: (schema, table, has USAGE, has SELECT).
const MISSING_GRANTS: &str = "
    SELECT n.nspname::text, c.relname::text,
           has_schema_privilege(r.oid, n.oid, 'USAGE'), has_table_privilege(r.oid, c.oid, 'SELECT')
      FROM pg_foreign_table ft
      JOIN pg_foreign_server s ON s.oid = ft.ftserver
      JOIN og_catalog.datasource d ON d.server_name = s.srvname
      JOIN pg_class c ON c.oid = ft.ftrelid
      JOIN pg_namespace n ON n.oid = c.relnamespace
      CROSS JOIN (SELECT oid FROM pg_roles WHERE rolname = $1) r
     WHERE NOT has_schema_privilege(r.oid, n.oid, 'USAGE') OR NOT has_table_privilege(r.oid, c.oid, 'SELECT')
     ORDER BY 1, 2";

fn needs_sync() -> bool {
    one::<bool>(
        &format!(
            "SELECT NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)
                 OR EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1 AND (rolcanlogin OR rolsuper OR rolinherit
                            OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls))
                 OR EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.member WHERE r.rolname = $1)
                 OR EXISTS (SELECT 1 FROM pg_db_role_setting s JOIN pg_roles r ON r.oid = s.setrole WHERE r.rolname = $1)
                 OR EXISTS ({MISSING_GRANTS})"
        ),
        &[READER.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(true)
}

/// After a datasource brings tables in: grant them to the reader. Only a
/// superuser can create or correct the role; anyone else's datasource is
/// granted on its first fabric query, or by `og_source_reader_sync()`.
pub fn after_import() {
    if unsafe { pg_sys::superuser() } {
        sync_reader();
    }
}

/// Create or correct `og_source_reader` and grant it the datasources' foreign
/// tables it cannot read yet. Fabric queries do this themselves when something
/// is missing; this is for an installer or an administrator.
#[pg_extern]
fn og_source_reader_sync() -> String {
    sync_reader()
}

// --------------------------------------------------- what a query may touch

thread_local! {
    /// While a fabric query runs: the relations it may read (the datasources'
    /// foreign tables). `None` the rest of the time.
    static ALLOWED: RefCell<Option<HashSet<u32>>> = const { RefCell::new(None) };
}

static mut PREV_CHECK_PERMS: pg_sys::ExecutorCheckPerms_hook_type = None;
static mut PREV_OBJECT_ACCESS: pg_sys::object_access_hook_type = None;
static mut HOOKED: bool = false;

/// Put the two checks in place, once per backend, after whatever was there.
fn install_hooks() {
    unsafe {
        if HOOKED {
            return;
        }
        PREV_CHECK_PERMS = pg_sys::ExecutorCheckPerms_hook;
        pg_sys::ExecutorCheckPerms_hook = Some(check_perms);
        PREV_OBJECT_ACCESS = pg_sys::object_access_hook;
        pg_sys::object_access_hook = Some(check_function);
        HOOKED = true;
    }
}

fn set_allowed(tables: Option<HashSet<u32>>) {
    ALLOWED.with(|a| *a.borrow_mut() = tables);
}

fn active() -> bool {
    ALLOWED.with(|a| a.borrow().is_some())
}

/// The first relation in `range_table` a fabric query may not read. A view
/// rewritten into a subquery keeps its `relid`, so a view is refused as well as
/// what it reads; every subquery, CTE and nested statement has its own range table
/// and passes through here too.
unsafe fn refused_relation(range_table: *mut pg_sys::List) -> Option<pg_sys::Oid> {
    if range_table.is_null() {
        return None;
    }
    ALLOWED.with(|a| {
        let a = a.borrow();
        let allowed = a.as_ref()?;
        let n = unsafe { (*range_table).length };
        (0..n).find_map(|i| {
            let rte = unsafe { pg_sys::list_nth(range_table, i) } as *mut pg_sys::RangeTblEntry;
            let relid = unsafe { (*rte).relid };
            (relid != pg_sys::Oid::INVALID && !allowed.contains(&relid.to_u32())).then_some(relid)
        })
    })
}

fn relation_name(relid: pg_sys::Oid) -> String {
    unsafe {
        let rel = take_cstr(pg_sys::get_rel_name(relid)).unwrap_or_else(|| relid.to_u32().to_string());
        match take_cstr(pg_sys::get_namespace_name(pg_sys::get_rel_namespace(relid))) {
            Some(ns) => format!("{ns}.{rel}"),
            None => rel,
        }
    }
}

fn refuse_relation(relid: pg_sys::Oid) -> ! {
    // No SQLSTATE in the message here: the query's handler adds it on the way out.
    pg_sys::panic::ErrorReport::new(
        PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
        format!("QUERY FABRIC reads datasource tables only — {} is not one", relation_name(relid)),
        "og_fabric_query",
    )
    .report(PgLogLevel::ERROR);
    unreachable!("an ERROR does not return")
}

#[cfg(not(any(feature = "pg13", feature = "pg14", feature = "pg15")))]
#[pg_guard]
unsafe extern "C-unwind" fn check_perms(range_table: *mut pg_sys::List, perm_infos: *mut pg_sys::List, ereport: bool) -> bool {
    unsafe {
        if let Some(prev) = PREV_CHECK_PERMS {
            if !prev(range_table, perm_infos, ereport) {
                return false;
            }
        }
        match refused_relation(range_table) {
            Some(relid) if ereport => refuse_relation(relid),
            Some(_) => false,
            None => true,
        }
    }
}

#[cfg(any(feature = "pg13", feature = "pg14", feature = "pg15"))]
#[pg_guard]
unsafe extern "C-unwind" fn check_perms(range_table: *mut pg_sys::List, ereport: bool) -> bool {
    unsafe {
        if let Some(prev) = PREV_CHECK_PERMS {
            if !prev(range_table, ereport) {
                return false;
            }
        }
        match refused_relation(range_table) {
            Some(relid) if ereport => refuse_relation(relid),
            Some(_) => false,
            None => true,
        }
    }
}

/// Every function an expression is about to call is announced here
/// (`InvokeFunctionExecuteHook`). In a fabric query, the server's own (`pg_*`)
/// and the engine's (`og_*`) are refused: no file reads, no locks, no signals, no
/// statistics, no way back into the graph.
#[pg_guard]
unsafe extern "C-unwind" fn check_function(
    access: pg_sys::ObjectAccessType::Type,
    class_id: pg_sys::Oid,
    object_id: pg_sys::Oid,
    sub_id: std::ffi::c_int,
    arg: *mut std::ffi::c_void,
) {
    unsafe {
        if let Some(prev) = PREV_OBJECT_ACCESS {
            prev(access, class_id, object_id, sub_id, arg);
        }
        if access != pg_sys::ObjectAccessType::OAT_FUNCTION_EXECUTE || !active() {
            return;
        }
        let name = take_cstr(pg_sys::get_func_name(object_id)).unwrap_or_default();
        if name.starts_with("pg_") || name.starts_with("og_") {
            ereport!(
                ERROR,
                PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
                format!("QUERY FABRIC reads datasource tables only — function {name}() is not available in it")
            );
        }
    }
}

// ------------------------------------------------------------------- query

struct Limits {
    datasource: Option<String>,
    max_rows: i64,
    max_bytes: i64,
}

fn limits(opts: &Map<String, Value>) -> Limits {
    for k in opts.keys() {
        if !matches!(k.as_str(), "datasource" | "max_rows" | "max_bytes") {
            raise(
                PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
                &format!("unknown option '{k}' — QUERY FABRIC takes datasource, max_rows, max_bytes (the time limit is og.statement_timeout)"),
            );
        }
    }
    let int = |k: &str| -> Option<i64> {
        match opts.get(k) {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_i64().filter(|n| *n > 0).unwrap_or_else(|| {
                raise(PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE, &format!("{k} must be a positive whole number, not {v}"))
            })),
        }
    };
    let datasource = match opts.get("datasource") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(v) => raise(PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE, &format!("datasource must be a name, not {v}")),
    };
    Limits {
        datasource,
        max_rows: int("max_rows").unwrap_or(DEFAULT_MAX_ROWS).min(CEILING_ROWS),
        max_bytes: int("max_bytes").unwrap_or(DEFAULT_MAX_BYTES).min(CEILING_BYTES),
    }
}

fn is_name(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// What the statement did, or why not, from inside the subtransaction.
enum Inner {
    Done(Value),
    TooBig,
}

/// Run a fabric query — see the module note. `opts`: datasource, max_rows, max_bytes.
pub fn query(sql: &str, opts: &Map<String, Value>) -> Value {
    let lim = limits(opts);
    let statement = check_statement(sql).unwrap_or_else(|e| refuse(&e));
    if statement.contains('\0') {
        refuse("the statement contains a NUL character");
    }

    let names: Vec<String> = Spi::connect(|c| {
        c.select("SELECT name::text FROM og_catalog.datasource ORDER BY name", None, &[])
            .map(|t| t.filter_map(|r| r.get::<String>(1).ok().flatten()).collect::<Vec<String>>())
            .unwrap_or_default()
    })
    .into_iter()
    .filter(|n| is_name(n))
    .collect();
    if let Some(ds) = &lim.datasource {
        if !names.contains(ds) {
            raise(
                PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
                &format!(
                    "datasource '{ds}' is not registered (registered: {})",
                    if names.is_empty() { "none".to_string() } else { names.join(", ") }
                ),
            );
        }
    }

    // Acting as the reader is for those who could become it anyway.
    if !unsafe { pg_sys::superuser() } {
        let may = one::<bool>(
            "SELECT pg_has_role(current_user, oid, 'MEMBER') FROM pg_roles WHERE rolname = $1",
            &[READER.into()],
        )
        .ok()
        .flatten()
        .unwrap_or(false);
        if !may {
            raise(
                PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
                &format!("permission denied for QUERY FABRIC — it runs as {READER}; grant that role to the caller"),
            );
        }
    }
    sync_reader();
    let reader = {
        let c = CString::new(READER).expect("no NUL");
        unsafe { pg_sys::get_role_oid(c.as_ptr(), false) }
    };
    let tables: HashSet<u32> = Spi::connect(|c| {
        c.select(
            "SELECT ft.ftrelid::int8 FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver
               JOIN og_catalog.datasource d ON d.server_name = s.srvname",
            None,
            &[],
        )
            .map(|t| t.filter_map(|r| r.get::<i64>(1).ok().flatten().map(|o| o as u32)).collect())
            .unwrap_or_default()
    });

    // unqualified names find the asked datasource's tables — or, with none asked, any datasource's
    let path: Vec<String> = match &lim.datasource {
        Some(ds) => vec![ident(ds)],
        None => names.iter().map(|n| ident(n)).collect(),
    };
    // The time limit is the statement's own (armed when the fabric statement began);
    // the query can read it as statement_timeout.
    let limit_ms = crate::cypher::timeout::configured_ms();
    let settings = format!(
        "SELECT set_config('transaction_read_only', 'on', true), set_config('search_path', {}, true){}",
        lit(&path.join(", ")),
        if limit_ms > 0 { format!(", set_config('statement_timeout', {}, true)", lit(&limit_ms.to_string())) } else { String::new() }
    );
    let wrapped = format!("{WRAP_HEAD}{statement}\n) AS og_fabric_sql LIMIT {}", lim.max_rows + 1);
    let head_chars = WRAP_HEAD.chars().count() as i64;
    let statement_chars = statement.chars().count() as i64;

    install_hooks();
    let started = std::time::Instant::now();
    let ctx = Ctx::save();
    unsafe {
        pg_sys::BeginInternalSubTransaction(std::ptr::null());
        pg_sys::MemoryContextSwitchTo(ctx.memory);
    }
    let restricted = ctx.sec
        | pg_sys::SECURITY_LOCAL_USERID_CHANGE as std::ffi::c_int
        | pg_sys::SECURITY_RESTRICTED_OPERATION as std::ffi::c_int;
    let body = std::panic::AssertUnwindSafe(|| unsafe {
        run_cursor(&settings, &wrapped, reader, restricted, tables, lim.max_rows, lim.max_bytes)
    });
    let guarded = std::panic::AssertUnwindSafe(&ctx);
    let inner = PgTryBuilder::new(body)
        .catch_others(move |e| {
            set_allowed(None);
            // the whole wrapper, not a field of it: a closure captures the place it names
            let ctx: &Ctx = *guarded;
            // The error is still on PostgreSQL's stack when it came from PostgreSQL:
            // take what pgrx's report lacks (the position, the exact SQLSTATE) before flushing it.
            let (state, code, message, detail, hint, position) = match &e {
                CaughtError::PostgresError(r) => {
                    let (raw, cursor, internal) = unsafe {
                        let ed = pg_sys::CopyErrorData();
                        let p = ((*ed).sqlerrcode, (*ed).cursorpos, (*ed).internalpos);
                        pg_sys::FreeErrorData(ed);
                        p
                    };
                    let pos = if internal > 0 { internal } else { cursor } as i64 - head_chars;
                    (
                        sqlstate(raw),
                        PgSqlErrorCode::from(raw),
                        r.message().to_string(),
                        r.detail().map(str::to_string),
                        r.hint().map(str::to_string),
                        (pos > 0 && pos <= statement_chars).then_some(pos),
                    )
                }
                CaughtError::ErrorReport(r) | CaughtError::RustPanic { ereport: r, .. } => (
                    sqlstate(r.sql_error_code() as i32),
                    r.sql_error_code(),
                    r.message().to_string(),
                    r.detail().map(str::to_string),
                    r.hint().map(str::to_string),
                    None,
                ),
            };
            unsafe { pg_sys::FlushErrorState() };
            ctx.restore();
            raise_as(code, &state, &message, position, detail, hint)
        })
        .execute();
    ctx.restore();

    match inner {
        Inner::TooBig => raise(
            PgSqlErrorCode::ERRCODE_PROGRAM_LIMIT_EXCEEDED,
            &format!(
                "the result is larger than {} (max_bytes) — select fewer columns or rows, or aggregate",
                human_bytes(lim.max_bytes)
            ),
        ),
        Inner::Done(mut v) => {
            if let Value::Object(m) = &mut v {
                m.insert("elapsed_ms".into(), json!((started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1000.0));
            }
            v
        }
    }
}

/// Where the caller was when the subtransaction began.
struct Ctx {
    user: pg_sys::Oid,
    sec: std::ffi::c_int,
    memory: pg_sys::MemoryContext,
    owner: pg_sys::ResourceOwner,
}

impl Ctx {
    fn save() -> Ctx {
        unsafe {
            let mut user = pg_sys::Oid::INVALID;
            let mut sec: std::ffi::c_int = 0;
            pg_sys::GetUserIdAndSecContext(&mut user, &mut sec);
            Ctx { user, sec, memory: pg_sys::CurrentMemoryContext, owner: pg_sys::CurrentResourceOwner }
        }
    }

    /// Back to the caller: its user, memory context and resource owner — and every
    /// setting and cursor of the subtransaction gone with the rollback.
    fn restore(&self) {
        set_allowed(None);
        unsafe {
            pg_sys::SetUserIdAndSecContext(self.user, self.sec);
            pg_sys::RollbackAndReleaseCurrentSubTransaction();
            pg_sys::MemoryContextSwitchTo(self.memory);
            pg_sys::CurrentResourceOwner = self.owner;
        }
    }
}

fn human_bytes(n: i64) -> String {
    if n >= 1 << 20 && n % (1 << 20) == 0 {
        format!("{} MB", n >> 20)
    } else {
        format!("{n} bytes")
    }
}

/// Inside the subtransaction: the settings, the switch to the reader, the
/// cursor, the rows. A PostgreSQL error here unwinds to `query`'s handler, which
/// rolls the subtransaction back (and with it the SPI connection and the cursor).
unsafe fn run_cursor(
    settings: &str,
    wrapped: &str,
    reader: pg_sys::Oid,
    restricted: std::ffi::c_int,
    tables: HashSet<u32>,
    max_rows: i64,
    max_bytes: i64,
) -> Inner {
    unsafe {
        if pg_sys::SPI_connect() < 0 {
            error!("SPI_connect failed");
        }
        let set = CString::new(settings).expect("no NUL in settings");
        if pg_sys::SPI_execute(set.as_ptr(), false, 0) < 0 {
            error!("could not apply the fabric query settings");
        }
        // From here on the statement's privileges are the reader's — a switch a
        // SET ROLE inside it cannot undo — and only datasource tables may be read.
        set_allowed(Some(tables));
        pg_sys::SetUserIdAndSecContext(reader, restricted);

        let src = CString::new(wrapped).expect("checked for NUL before");
        let portal = pg_sys::SPI_cursor_open_with_args(
            std::ptr::null(),
            src.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
            false,
            0,
        );
        let mut columns: Vec<String> = Vec::new();
        let mut types: Vec<String> = Vec::new();
        let mut oids: Vec<u32> = Vec::new();
        let mut rows: Vec<Value> = Vec::new();
        let mut bytes: i64 = 0;
        let mut truncated = false;
        let want = max_rows + 1;
        'fetch: loop {
            let ask = BATCH.min(want - rows.len() as i64);
            if ask <= 0 {
                break;
            }
            pg_sys::SPI_cursor_fetch(portal, true, ask as std::ffi::c_long);
            let table = pg_sys::SPI_tuptable;
            let got = pg_sys::SPI_processed as usize;
            if table.is_null() {
                break;
            }
            let desc = (*table).tupdesc;
            let natts = (*desc).natts;
            if columns.is_empty() && natts > 0 {
                for i in 1..=natts {
                    columns.push(take_cstr(pg_sys::SPI_fname(desc, i)).unwrap_or_default());
                    let oid = pg_sys::SPI_gettypeid(desc, i);
                    oids.push(oid.to_u32());
                    types.push(take_cstr(pg_sys::format_type_be(oid)).unwrap_or_default());
                }
            }
            for r in 0..got {
                pgrx::check_for_interrupts!();
                if rows.len() as i64 == max_rows {
                    truncated = true;
                    break 'fetch;
                }
                let tuple = *(*table).vals.add(r);
                let mut row = Vec::with_capacity(natts as usize);
                for i in 1..=natts {
                    match take_cstr(pg_sys::SPI_getvalue(tuple, desc, i)) {
                        None => {
                            bytes += 4;
                            row.push(Value::Null);
                        }
                        Some(text) => {
                            bytes += text.len() as i64 + 1;
                            row.push(typed(oids[(i - 1) as usize], text));
                        }
                    }
                }
                rows.push(Value::Array(row));
                if bytes > max_bytes {
                    pg_sys::SPI_cursor_close(portal);
                    pg_sys::SPI_finish();
                    return Inner::TooBig;
                }
            }
            pg_sys::SPI_freetuptable(table);
            if (got as i64) < ask {
                break;
            }
        }
        pg_sys::SPI_cursor_close(portal);
        pg_sys::SPI_finish();
        let n = rows.len();
        Inner::Done(json!({
            "columns": columns,
            "types": types,
            "rows": rows,
            "row_count": n,
            "truncated": truncated,
            "bytes": bytes,
        }))
    }
}

/// A palloc'd C string as a Rust string, freed.
unsafe fn take_cstr(p: *mut std::ffi::c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    unsafe {
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        pg_sys::pfree(p.cast());
        Some(s)
    }
}

/// What `QUERY FABRIC` translates to (as `SHOW COLUMN STATS` translates to
/// `og_fabric_column_stats`). Like every engine function it is not PUBLIC's.
#[pg_extern(volatile, parallel_unsafe)]
fn og_fabric_query(sql: &str, options: default!(JsonB, "'{}'")) -> JsonB {
    let opts = match options.0 {
        Value::Object(m) => m,
        Value::Null => Map::new(),
        other => raise(PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE, &format!("options must be a JSON object, not {other}")),
    };
    JsonB(query(sql, &opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_that_pass() {
        for sql in [
            "SELECT 1", "select 1;", "  WITH x AS (SELECT 1 AS a) SELECT a FROM x ;; ", "(SELECT 1) UNION (SELECT 2)",
            "SELECT 'delete; drop table x' AS note", "SELECT \"update\" FROM t", "SELECT E'it\\'s; insert' AS s",
            "SELECT $$ ; delete $$ AS s", "SELECT $tag$ ) ( ; $tag$ AS s", "SELECT 1 -- ; drop table t",
            "SELECT /* insert /* nested */ ; */ 1", "SELECT replace(name, 'a', 'b') FROM t",
            "SELECT a$b FROM t WHERE x = $1", "SELECT U&'d\\0061t' AS s",
        ] {
            assert!(check_statement(sql).is_ok(), "{sql}: {:?}", check_statement(sql));
        }
        assert_eq!(check_statement("  SELECT 1 ;  ").unwrap(), "SELECT 1");
    }

    #[test]
    fn statements_refused() {
        for (sql, why) in [
            ("", "empty"), ("DELETE FROM t", "SELECT or WITH"), ("UPDATE t SET a = 1", "SELECT or WITH"),
            ("SELECT 1; DROP TABLE t", "one statement"), ("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d", "DELETE"),
            ("SELECT * INTO t2 FROM t", "INTO"), ("SELECT * FROM t FOR UPDATE", "UPDATE"), ("TABLE t", "SELECT or WITH"),
            ("SELECT 1) AS q --", "parentheses"), ("SELECT (1", "parentheses"), ("SELECT 'open", "string"),
            ("SELECT /* open", "comment"), ("SELECT $x$ open", "$"), ("VALUES (1)", "SELECT or WITH"),
            ("SET ROLE og", "SELECT or WITH"),
        ] {
            let e = check_statement(sql).expect_err(sql);
            assert!(e.contains(why), "{sql}: {e}");
        }
    }

    #[test]
    fn numbers_keep_their_digits() {
        assert_eq!(exact_number("4.50"), json!(4.5));
        assert_eq!(exact_number("9007199254740993"), json!("9007199254740993"));
        assert_eq!(exact_number("123456789012345"), json!(123456789012345_i64));
        assert_eq!(exact_number("NaN"), json!("NaN"));
        assert_eq!(finite("Infinity"), json!("Infinity"));
        assert_eq!(finite("0.1"), json!(0.1));
        assert_eq!(typed(16, "t".into()), json!(true));
        assert_eq!(typed(1082, "2020-01-02".into()), json!("2020-01-02"));
        assert_eq!(typed(3802, "{\"a\": [1]}".into()), json!({"a": [1]}));
    }

    #[test]
    fn sqlstates() {
        assert_eq!(sqlstate(PgSqlErrorCode::ERRCODE_UNDEFINED_COLUMN as i32), "42703");
        assert_eq!(sqlstate(PgSqlErrorCode::ERRCODE_QUERY_CANCELED as i32), "57014");
        assert_eq!(tag("42703", Some(8)), "[SQLSTATE 42703, position 8]");
    }
}
