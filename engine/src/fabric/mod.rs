//! Virtual fabric — external tables as graph types, with no copy (experiment).
//!
//! The question this module answers is whether a Cypher-native graph can also be
//! the data-fabric layer, so that a client never has to notice which labels are
//! stored here and which live in another system.
//!
//! It leans on the same observation as spec 005: a compiled Cypher query reads
//! ordinary relations. A foreign table is an ordinary relation to the planner,
//! so a virtual type is just spec 005's mapping view laid over a foreign table.
//! Filters, sorts, limits — and joins between two tables on the same server —
//! are then pushed to the source by the FDW, which already knows how, and the
//! cross-source joins nobody can push run in PostgreSQL's own executor.
//!
//! Three objects, mirroring MindsDB's surface:
//!
//! | MindsDB                                   | here                       |
//! |-------------------------------------------|----------------------------|
//! | `CREATE DATABASE erp WITH ENGINE=postgres`| `og_create_datasource`     |
//! | a table of that database                  | `og_map_virtual` → a label |
//! | `CREATE VIEW … JOIN …` across databases   | `og_create_virtual_rel`    |
//!
//! The last row is where the two differ. MindsDB federates by letting the user
//! write the join; here the join is declared once, as a relationship type, and
//! every Cypher hop over it becomes that join.

use crate::catalog::types;
use crate::spiu::{ident, lit, one};
use pgrx::prelude::*;

pub mod sqlish;
use pgrx::JsonB;
use serde_json::{json, Map, Value};

/// 36 bits: the local part of a node identifier (see `id.rs`).
const LOCAL_MASK: i64 = (1_i64 << 36) - 1;

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

fn server_name(ds: &str) -> String {
    format!("og_ds_{}", sanitize(ds))
}

fn param(params: &Map<String, Value>, k: &str) -> Option<String> {
    params.get(k).map(|v| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn need(params: &Map<String, Value>, k: &str) -> String {
    param(params, k).unwrap_or_else(|| error!("datasource parameter '{k}' is required"))
}

/// Register an external database. `engine` is `postgres`, `mysql`, or
/// `mindsdb` (MindsDB's MySQL wire endpoint, so that MindsDB itself can be the
/// thing a virtual type points at).
///
/// `params`: `host`, `port`, `database` (postgres only), `user`, `password`.
/// The password goes into a user mapping and nowhere else.
#[pg_extern]
fn og_create_datasource(name: &str, engine: &str, params: JsonB) -> String {
    let Value::Object(p) = params.0 else { error!("params must be a JSON object") };
    check_datasource_name(name);
    if let Some(types) = dependent_types(&server_name(name)) {
        error!(
            "datasource '{name}' backs virtual types ({types}); replacing it would drop them. \
             Drop those first, or materialize them"
        );
    }
    let srv = server_name(name);
    let engine = engine.to_ascii_lowercase();
    let host = need(&p, "host");
    let user = need(&p, "user");
    let password = param(&p, "password").unwrap_or_default();

    let (fdw, server_opts, mapping_opts) = match engine.as_str() {
        "postgres" | "postgresql" => {
            let port = param(&p, "port").unwrap_or_else(|| "5432".into());
            let db = need(&p, "database");
            (
                "postgres_fdw",
                format!(
                    "host {}, port {}, dbname {}, use_remote_estimate 'true', fetch_size '1000'",
                    lit(&host), lit(&port), lit(&db)
                ),
                format!("user {}, password {}", lit(&user), lit(&password)),
            )
        }
        "mysql" | "mindsdb" => {
            let port = param(&p, "port")
                .unwrap_or_else(|| if engine == "mindsdb" { "47335".into() } else { "3306".into() });
            (
                "mysql_fdw",
                format!("host {}, port {}", lit(&host), lit(&port)),
                format!("username {}, password {}", lit(&user), lit(&password)),
            )
        }
        other => error!("unsupported datasource engine '{other}': postgres, mysql, mindsdb"),
    };

    Spi::run(&format!("CREATE EXTENSION IF NOT EXISTS {fdw}"))
        .unwrap_or_else(|e| error!("{fdw} is not available: {e}"));
    Spi::run(&format!("DROP SERVER IF EXISTS {} CASCADE", ident(&srv))).ok();
    Spi::run(&format!(
        "CREATE SERVER {} FOREIGN DATA WRAPPER {fdw} OPTIONS ({server_opts})",
        ident(&srv)
    ))
    .unwrap_or_else(|e| error!("failed to create server for '{name}': {e}"));
    Spi::run(&format!(
        "CREATE USER MAPPING FOR PUBLIC SERVER {} OPTIONS ({mapping_opts})",
        ident(&srv)
    ))
    .unwrap_or_else(|e| error!("failed to create user mapping for '{name}': {e}"));

    let mut shown = p.clone();
    shown.remove("password");
    Spi::run_with_args(
        "INSERT INTO og_catalog.datasource (name, engine, server_name, options)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (name) DO UPDATE SET engine = EXCLUDED.engine,
             server_name = EXCLUDED.server_name, options = EXCLUDED.options",
        &[name.into(), engine.as_str().into(), srv.as_str().into(), JsonB(Value::Object(shown)).into()],
    )
    .expect("datasource registration failed");

    // A datasource that names its schema (PostgreSQL) or database (MySQL) is
    // imported whole, into a local schema of the datasource's own name — so its
    // tables read as `erp.supplier`, the way MindsDB addresses them.
    let imported = match default_remote_schema(&engine, &p) {
        Some(remote) => {
            let local = ident(name);
            let ours = one::<bool>(
                "SELECT NOT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                                     WHERE n.nspname = $1 AND c.relkind <> 'f')",
                &[name.into()],
            )
            .ok()
            .flatten()
            .unwrap_or(false);
            if !ours {
                error!("a schema named '{name}' already holds local tables; choose another datasource name");
            }
            Spi::run(&format!("CREATE SCHEMA IF NOT EXISTS {local}")).expect("schema");
            Spi::run(&format!(
                "IMPORT FOREIGN SCHEMA {} FROM SERVER {} INTO {local}",
                ident(&remote), ident(&srv)
            ))
            .unwrap_or_else(|e| error!("failed to import '{remote}' from '{name}': {e}"));
            protect_server(&srv);
            one::<i64>(
                "SELECT count(*) FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver
                  WHERE s.srvname = $1",
                &[srv.as_str().into()],
            )
            .ok()
            .flatten()
        }
        None => None,
    };
    match imported {
        Some(n) => format!("datasource {name} ({engine}): {n} tables as {name}.*"),
        None => format!("datasource {name} ({engine}): no schema named, import tables with og_import_foreign"),
    }
}

/// Give every foreign table of a server the trigger that refuses writes, where
/// it does not have it yet.
fn protect_server(server: &str) {
    let tables: Vec<String> = Spi::connect(|c| {
        c.select(
            "SELECT format('%I.%I', n.nspname, c.relname) FROM pg_foreign_table ft
               JOIN pg_foreign_server s ON s.oid = ft.ftserver
               JOIN pg_class c ON c.oid = ft.ftrelid JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE s.srvname = $1
                AND NOT EXISTS (SELECT 1 FROM pg_trigger t WHERE t.tgrelid = c.oid AND t.tgname = 'og_read_only')",
            None,
            &[server.into()],
        )
        .expect("foreign table scan failed")
        .filter_map(|r| r.get::<String>(1).ok().flatten())
        .collect()
    });
    for t in tables {
        Spi::run(&format!(
            "CREATE TRIGGER og_read_only BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON {t} \
             FOR EACH STATEMENT EXECUTE FUNCTION og_catalog.og_refuse_source_write()"
        ))
        .unwrap_or_else(|e| error!("failed to protect {t}: {e}"));
    }
}

/// The remote schema a datasource is about, if its parameters name one.
fn default_remote_schema(engine: &str, p: &Map<String, Value>) -> Option<String> {
    match engine {
        "postgres" | "postgresql" => param(p, "schema"),
        "mysql" => param(p, "database"),
        _ => None,
    }
}

fn check_datasource_name(name: &str) {
    let ok = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    let reserved = ["public", "og_catalog", "og_data", "information_schema"];
    if !ok || reserved.contains(&name) || name.starts_with("pg_") || name.starts_with("og_") {
        error!("'{name}' cannot name a datasource: use lower-case letters, digits and '_', and not a system schema");
    }
}

/// Virtual types whose storage reads this server's foreign tables — directly, or
/// through views built on them. Dropping the server would take them along.
fn dependent_types(server: &str) -> Option<String> {
    one::<String>(
        "WITH RECURSIVE dep(oid) AS (
             SELECT ft.ftrelid FROM pg_foreign_table ft
               JOIN pg_foreign_server s ON s.oid = ft.ftserver WHERE s.srvname = $1
             UNION
             SELECT r.ev_class FROM dep
               JOIN pg_depend d ON d.refobjid = dep.oid AND d.classid = 'pg_rewrite'::regclass
               JOIN pg_rewrite r ON r.oid = d.objid)
         SELECT string_agg(DISTINCT t.name, ', ' ORDER BY t.name)
           FROM dep JOIN pg_class c ON c.oid = dep.oid AND c.relkind = 'v'
           JOIN pg_namespace n ON n.oid = c.relnamespace
           JOIN og_catalog.type t ON t.storage_table = n.nspname || '.' || c.relname",
        &[server.into()],
    )
    .ok()
    .flatten()
}

/// Forget a datasource: its server, its foreign tables and its local schema.
/// Refused while virtual types still read from it.
#[pg_extern]
fn og_drop_datasource(name: &str) -> String {
    let srv = one::<String>("SELECT server_name FROM og_catalog.datasource WHERE name = $1", &[name.into()])
        .ok()
        .flatten()
        .unwrap_or_else(|| error!("datasource '{name}' is not registered"));
    if let Some(types) = dependent_types(&srv) {
        error!("datasource '{name}' still backs virtual types ({types}); drop or materialize them first");
    }
    Spi::run(&format!("DROP SERVER IF EXISTS {} CASCADE", ident(&srv)))
        .unwrap_or_else(|e| error!("failed to drop server of '{name}': {e}"));
    // Only ever held foreign tables, which went with the server.
    Spi::run(&format!("DROP SCHEMA IF EXISTS {} RESTRICT", ident(name))).ok();
    Spi::run_with_args("DELETE FROM og_catalog.datasource WHERE name = $1", &[name.into()])
        .expect("datasource removal failed");
    format!("datasource {name} dropped")
}

/// Pick up tables added at the source since the datasource was created.
#[pg_extern]
fn og_refresh_datasource(name: &str) -> String {
    let (srv, engine, opts) = Spi::connect(|c| {
        c.select("SELECT server_name, engine, options FROM og_catalog.datasource WHERE name = $1", Some(1), &[name.into()])
            .ok()?
            .next()
            .and_then(|r| Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok()??, r.get::<JsonB>(3).ok()??)))
    })
    .unwrap_or_else(|| error!("datasource '{name}' is not registered"));
    let Value::Object(p) = opts.0 else { error!("datasource options are not an object") };
    let remote = default_remote_schema(&engine, &p)
        .unwrap_or_else(|| error!("datasource '{name}' names no schema to refresh"));
    let have: Vec<String> = Spi::connect(|c| {
        c.select(
            "SELECT c.relname::text FROM pg_foreign_table ft JOIN pg_class c ON c.oid = ft.ftrelid
               JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1",
            None,
            &[name.into()],
        )
        .expect("foreign table scan failed")
        .filter_map(|r| r.get::<String>(1).ok().flatten())
        .collect()
    });
    let except = if have.is_empty() {
        String::new()
    } else {
        format!(" EXCEPT ({})", have.iter().map(|t| ident(t)).collect::<Vec<_>>().join(", "))
    };
    Spi::run(&format!(
        "IMPORT FOREIGN SCHEMA {}{except} FROM SERVER {} INTO {}",
        ident(&remote), ident(&srv), ident(name)
    ))
    .unwrap_or_else(|e| error!("failed to refresh '{name}': {e}"));
    protect_server(&srv);
    let now = one::<i64>(
        "SELECT count(*) FROM pg_foreign_table ft JOIN pg_foreign_server s ON s.oid = ft.ftserver WHERE s.srvname = $1",
        &[srv.as_str().into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0);
    format!("datasource {name}: {} new tables, {now} in all", now - have.len() as i64)
}


/// Local schema that holds the foreign tables of one remote schema.
fn local_schema(ds: &str, remote_schema: &str) -> String {
    let default = one::<String>(
        "SELECT CASE WHEN engine IN ('postgres', 'postgresql') THEN options->>'schema'
                     WHEN engine = 'mysql' THEN options->>'database' END
           FROM og_catalog.datasource WHERE name = $1",
        &[ds.into()],
    )
    .ok()
    .flatten();
    if default.as_deref() == Some(remote_schema) {
        return ds.to_string();
    }
    format!("og_ext_{}__{}", sanitize(ds), sanitize(remote_schema))
}

/// The ontology type a remote column type maps to, and the cast to apply.
fn og_type_of(pg_type: &str) -> &'static str {
    match pg_type {
        "smallint" | "integer" => "int",
        "bigint" => "long",
        "numeric" => "numeric",
        "real" | "double precision" => "float",
        "boolean" => "bool",
        "date" => "date",
        t if t.starts_with("timestamp") => "datetime",
        "json" | "jsonb" => "jsonb",
        _ => "string",
    }
}

fn foreign_columns(schema: &str, table: &str) -> Vec<(String, String)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT column_name::text, data_type::text FROM information_schema.columns
                  WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                None,
                &[schema.into(), table.into()],
            )
            .expect("column lookup failed")
            .filter_map(|r| Some((r.get::<String>(1).ok()??, r.get::<String>(2).ok()??)))
            .collect()
    })
}

/// Bring one remote table in as a foreign table, columns and types read from
/// the source, and return its local name. Building block for views that give a
/// remote table a key it lacks, or join several (see `og_map_relation`).
#[pg_extern]
fn og_import_foreign(datasource: &str, remote: &str) -> String {
    let (s, t) = import_foreign(datasource, remote);
    format!("{s}.{t}")
}

fn import_foreign(datasource: &str, remote: &str) -> (String, String) {
    let srv = one::<String>(
        "SELECT server_name FROM og_catalog.datasource WHERE name = $1",
        &[datasource.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| error!("datasource '{datasource}' is not registered"));
    let (rschema, rtable) = remote
        .split_once('.')
        .unwrap_or_else(|| error!("remote must be 'schema.table', got '{remote}'"));

    let lschema = local_schema(datasource, rschema);
    Spi::run(&format!("CREATE SCHEMA IF NOT EXISTS {}", ident(&lschema))).expect("schema");
    // Already imported: keep it. Views built on it (og_map_relation) depend on
    // it, and dropping it would take them along.
    let exists = one::<bool>(
        "SELECT to_regclass($1) IS NOT NULL",
        &[format!("{}.{}", ident(&lschema), ident(rtable)).into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if exists {
        return (lschema, rtable.to_string());
    }
    Spi::run(&format!(
        "IMPORT FOREIGN SCHEMA {} LIMIT TO ({}) FROM SERVER {} INTO {}",
        ident(rschema), ident(rtable), ident(&srv), ident(&lschema)
    ))
    .unwrap_or_else(|e| error!("failed to import {remote} from '{datasource}': {e}"));
    protect_server(&srv);
    (lschema, rtable.to_string())
}

/// Expose a remote table as a node type.
///
/// `remote` is `schema.table` (for MySQL the schema is the database). The type
/// is created if it does not exist, and every remote column becomes a property
/// unless `property_map` (`{"property": "column"}`) picks a subset or renames.
///
/// Node identity is derived from `key_column`. An integer key is used as is; any
/// other key is hashed into the 36-bit local part, so two keys *can* collide —
/// at a million rows the chance is about one in 140,000. A virtual node's id is
/// a handle for this query, not a durable reference; the key property is.
#[pg_extern]
fn og_map_virtual(
    graph: &str,
    type_name: &str,
    datasource: &str,
    remote: &str,
    key_column: &str,
    property_map: default!(Option<JsonB>, "NULL"),
) -> JsonB {
    let (lschema, rtable) = import_foreign(datasource, remote);
    let mut out = map_relation(graph, type_name, &lschema, &rtable, key_column, property_map);
    if let Value::Object(m) = &mut out.0 {
        m.insert("datasource".into(), Value::String(datasource.into()));
    }
    out
}

/// Expose any local relation — a foreign table created by hand, or a view that
/// joins several of them (MindsDB's "virtual view") — as a node type.
#[pg_extern]
fn og_map_relation(
    graph: &str,
    type_name: &str,
    relation: &str,
    key_column: &str,
    property_map: default!(Option<JsonB>, "NULL"),
) -> JsonB {
    let (schema, rel) = relation
        .split_once('.')
        .unwrap_or_else(|| error!("relation must be 'schema.name', got '{relation}'"));
    map_relation(graph, type_name, schema, rel, key_column, property_map)
}

pub(crate) fn map_relation(
    graph: &str,
    type_name: &str,
    lschema: &str,
    rtable: &str,
    key_column: &str,
    property_map: Option<JsonB>,
) -> JsonB {
    let remote = format!("{lschema}.{rtable}");
    let cols = foreign_columns(lschema, rtable);
    if cols.is_empty() {
        error!("'{remote}' has no columns, or does not exist");
    }
    let key_type = cols
        .iter()
        .find(|(c, _)| c == key_column)
        .map(|(_, t)| t.clone())
        .unwrap_or_else(|| error!("key column '{key_column}' is not a column of '{remote}'"));

    // 2. Which properties, from which columns.
    let picks: Vec<(String, String, String)> = match property_map.map(|j| j.0) {
        Some(Value::Object(m)) => m
            .iter()
            .map(|(prop, col)| {
                let col = col.as_str().unwrap_or_else(|| error!("property_map values are column names"));
                let t = cols
                    .iter()
                    .find(|(c, _)| c == col)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| error!("'{col}' is not a column of '{remote}'"));
                (prop.clone(), col.to_string(), t)
            })
            .collect(),
        _ => cols.iter().map(|(c, t)| (c.clone(), c.clone(), t.clone())).collect(),
    };

    // 3. The type and its properties, declared while its table is still a table.
    let gid = types::graph_id(graph);
    let tid = match types::try_type_id(gid, type_name) {
        Some(t) => t,
        None => types::create_type_inner(graph, type_name, "entity", &[], false),
    };
    for (prop, _, rtype) in &picks {
        let declared = one::<String>(
            "SELECT data_type FROM og_catalog.property WHERE type_id = $1 AND name = $2",
            &[tid.into(), prop.as_str().into()],
        )
        .ok()
        .flatten();
        if declared.is_none() {
            Spi::run_with_args(
                "SELECT og_add_property($1, $2, $3, $4)",
                &[graph.into(), type_name.into(), prop.as_str().into(), og_type_of(rtype).into()],
            )
            .unwrap_or_else(|e| error!("failed to declare {type_name}.{prop}: {e}"));
        }
    }

    // 4. Replace the type's table with a view over the foreign table.
    let ftable = format!("{}.{}", ident(lschema), ident(rtable));
    let key = ident(key_column);
    let id_expr = match key_type.as_str() {
        "smallint" | "integer" | "bigint" => format!("og_make_id(0, {tid}, ({key})::int8)"),
        _ => format!("og_make_id(0, {tid}, hashtextextended(({key})::text, 0) & {LOCAL_MASK})"),
    };
    let mut select = vec![format!("{id_expr} AS id")];
    for (prop, col, _) in &picks {
        let dtype = one::<String>(
            "SELECT data_type FROM og_catalog.property WHERE type_id = $1 AND name = $2",
            &[tid.into(), prop.as_str().into()],
        )
        .ok()
        .flatten()
        .unwrap_or_else(|| "text".into());
        select.push(format!("({})::{dtype} AS {}", ident(col), types::column_name(prop)));
    }
    select.push("NULL::jsonb AS __ext".into());

    let table = types::storage_table(tid)
        .unwrap_or_else(|| error!("'{type_name}' is abstract and cannot be virtual"));
    // The type's storage may already be a view from an earlier mapping.
    let is_view = one::<bool>("SELECT relkind = 'v' FROM pg_class WHERE oid = $1::regclass", &[table.as_str().into()])
        .ok()
        .flatten()
        .unwrap_or(false);
    let kind = if is_view { "VIEW" } else { "TABLE" };
    Spi::run(&format!("DROP {kind} IF EXISTS {table} CASCADE"))
        .unwrap_or_else(|e| error!("failed to drop storage of '{type_name}': {e}"));
    Spi::run(&format!(
        "CREATE VIEW {table}{} AS SELECT {} FROM {ftable}",
        crate::spiu::VIEW_SECURITY,
        select.join(", ")
    ))
    .unwrap_or_else(|e| error!("failed to create virtual view for '{type_name}': {e}"));
    crate::catalog::privileges::apply_to_view(&table);

    let pmap: Map<String, Value> =
        picks.iter().map(|(p, c, _)| (p.clone(), Value::String(c.clone()))).collect();
    Spi::run_with_args(
        "INSERT INTO og_catalog.mapping (type_id, source_table, id_column, property_map, writable)
         VALUES ($1, $2, $3, $4, false)
         ON CONFLICT (type_id) DO UPDATE
            SET source_table = EXCLUDED.source_table, id_column = EXCLUDED.id_column,
                property_map = EXCLUDED.property_map",
        &[tid.into(), ftable.as_str().into(), key_column.into(), JsonB(Value::Object(pmap.clone())).into()],
    )
    .expect("mapping registration failed");
    crate::catalog::labeling::bump_schema_version(gid, &format!("virtual type {type_name} → {remote}"));

    JsonB(json!({
        "type": type_name, "type_id": tid,
        "foreign_table": ftable, "key": key_column, "properties": pmap,
    }))
}

/// Declare `(a:src_type)-[:rel_name]->(b:dst_type)` to hold exactly when
/// `a.src_prop = b.dst_prop`. Either side may be virtual or native, which is the
/// point: a node stored here can reach rows that are not.
#[pg_extern]
fn og_create_virtual_rel(
    graph: &str,
    rel_name: &str,
    src_type: &str,
    src_prop: &str,
    dst_type: &str,
    dst_prop: &str,
) -> i32 {
    let gid = types::graph_id(graph);
    let s = types::type_id(gid, src_type);
    let d = types::type_id(gid, dst_type);
    for (t, tn, p) in [(s, src_type, src_prop), (d, dst_type, dst_prop)] {
        let known = one::<bool>(
            "SELECT true FROM og_catalog.property WHERE type_id = $1 AND name = $2",
            &[t.into(), p.into()],
        )
        .ok()
        .flatten()
        .unwrap_or(false);
        if !known {
            error!("'{tn}' has no declared property '{p}' to join on");
        }
    }
    let rid = match types::try_type_id(gid, rel_name) {
        Some(r) => r,
        None => types::create_type_inner(graph, rel_name, "relation", &[], false),
    };
    Spi::run_with_args(
        "INSERT INTO og_catalog.virtual_rel (type_id, src_type, src_prop, dst_type, dst_prop)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (type_id) DO UPDATE SET src_type = EXCLUDED.src_type,
             src_prop = EXCLUDED.src_prop, dst_type = EXCLUDED.dst_type, dst_prop = EXCLUDED.dst_prop",
        &[rid.into(), s.into(), src_prop.into(), d.into(), dst_prop.into()],
    )
    .expect("virtual relationship registration failed");
    crate::catalog::labeling::bump_schema_version(
        gid,
        &format!("virtual relationship {src_type}.{src_prop} -[{rel_name}]-> {dst_type}.{dst_prop}"),
    );
    rid
}

/// A virtual relationship's definition, if `rel_tid` is one.
pub struct VirtualRel {
    pub src_type: i32,
    pub src_prop: String,
    pub dst_type: i32,
    pub dst_prop: String,
}

pub fn virtual_rel(rel_tid: i32) -> Option<VirtualRel> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT src_type, src_prop, dst_type, dst_prop FROM og_catalog.virtual_rel
                  WHERE type_id = $1",
                Some(1),
                &[rel_tid.into()],
            )
            .ok()?
            .next()
            .and_then(|r| {
                Some(VirtualRel {
                    src_type: r.get::<i32>(1).ok()??,
                    src_prop: r.get::<String>(2).ok()??,
                    dst_type: r.get::<i32>(3).ok()??,
                    dst_prop: r.get::<String>(4).ok()??,
                })
            })
    })
}

/// What a datasource-backed graph looks like, for an agent deciding what to ask.
#[pg_extern]
fn og_fabric_catalog(graph: &str) -> JsonB {
    let gid = types::graph_id(graph);
    let v = one::<JsonB>(
        "SELECT jsonb_build_object(
            'datasources', (SELECT coalesce(jsonb_agg(jsonb_build_object('name', name, 'engine', engine,
                                    'options', options)), '[]') FROM og_catalog.datasource),
            'virtual_types', (SELECT coalesce(jsonb_agg(jsonb_build_object('type', t.name,
                                    'source', m.source_table, 'key', m.id_column)), '[]')
                                FROM og_catalog.mapping m JOIN og_catalog.type t USING (type_id)
                                JOIN pg_class c ON c.oid = to_regclass(t.storage_table)
                               WHERE t.graph_id = $1 AND c.relkind = 'v'),
            'materialized_types', (SELECT coalesce(jsonb_agg(jsonb_build_object('type', t.name,
                                    'copied_from', m.source_table)), '[]')
                                FROM og_catalog.mapping m JOIN og_catalog.type t USING (type_id)
                                JOIN pg_class c ON c.oid = to_regclass(t.storage_table)
                               WHERE t.graph_id = $1 AND c.relkind <> 'v'),
            'virtual_rels', (SELECT coalesce(jsonb_agg(jsonb_build_object('rel', r.name,
                                    'from', s.name || '.' || v.src_prop, 'to', d.name || '.' || v.dst_prop)), '[]')
                               FROM og_catalog.virtual_rel v
                               JOIN og_catalog.type r ON r.type_id = v.type_id
                               JOIN og_catalog.type s ON s.type_id = v.src_type
                               JOIN og_catalog.type d ON d.type_id = v.dst_type
                              WHERE r.graph_id = $1))",
        &[gid.into()],
    )
    .ok()
    .flatten();
    v.unwrap_or(JsonB(json!({})))
}

/// Where a virtual type's rows live, if `tid` is one: its storage is a view
/// laid over a mapped relation rather than a table of its own.
pub fn virtual_source(tid: i32) -> Option<String> {
    one::<String>(
        "SELECT m.source_table FROM og_catalog.mapping m
           JOIN og_catalog.type t USING (type_id)
           JOIN pg_class c ON c.oid = to_regclass(t.storage_table)
          WHERE m.type_id = $1 AND c.relkind = 'v'",
        &[tid.into()],
    )
    .ok()
    .flatten()
}

/// The datasource a foreign table belongs to, by its local name.
///
/// postgres_fdw names the local foreign table in a plan; mysql_fdw names the
/// remote `database.table`, so that is tried second, against the table options.
fn datasource_of(relation: &str) -> Option<String> {
    let by_local = one::<String>(
        "SELECT d.name FROM pg_foreign_table ft
           JOIN pg_foreign_server s ON s.oid = ft.ftserver
           JOIN og_catalog.datasource d ON d.server_name = s.srvname
          WHERE ft.ftrelid = to_regclass($1)",
        &[relation.into()],
    )
    .ok()
    .flatten();
    if by_local.is_some() {
        return by_local;
    }
    let (db, table) = relation.split_once('.')?;
    one::<String>(
        "SELECT d.name FROM pg_foreign_table ft
           JOIN pg_foreign_server s ON s.oid = ft.ftserver
           JOIN og_catalog.datasource d ON d.server_name = s.srvname
          WHERE ft.ftoptions @> ARRAY['dbname=' || $1, 'table_name=' || $2]
          ORDER BY d.name LIMIT 1",
        &[db.into(), table.into()],
    )
    .ok()
    .flatten()
}

/// What a Cypher query sends to each source, and what is left to do here.
///
/// One row per step of the plan that matters to federation: a `remote` step is
/// the SQL a source receives (a scan, or a whole join when both tables live on
/// one server); a `local` step is a join PostgreSQL performs on what came back.
/// With `analyze`, the query is run and `rows` is how many rows each step
/// produced — for a remote step, how many crossed the wire.
#[pg_extern]
fn og_fabric_explain(
    graph: &str,
    query: &str,
    analyze: default!(bool, false),
    params: default!(JsonB, "'{}'"),
) -> TableIterator<
    'static,
    (
        name!(step, i32),
        name!(place, String),
        name!(operation, String),
        name!(detail, String),
        name!(rows, Option<i64>),
    ),
> {
    let (sql, _) = crate::cypher::compile_cached(graph, query).unwrap_or_else(|e| error!("{e}"));
    let opts = if analyze { "ANALYZE, VERBOSE, FORMAT JSON" } else { "VERBOSE, FORMAT JSON" };
    // EXPLAIN's JSON format is `json`, not `jsonb`.
    let plan = crate::spiu::one_mut::<pgrx::Json>(&format!("EXPLAIN ({opts}) {sql}"), &[params.into()])
        .unwrap_or_else(|e| error!("{e}"))
        .map(|j| j.0)
        .unwrap_or(Value::Null);

    let mut out: Vec<(i32, String, String, String, Option<i64>)> = Vec::new();
    fn walk(n: &Value, out: &mut Vec<(i32, String, String, String, Option<i64>)>) {
        // Children first: the sources are read before anything joins them.
        if let Some(Value::Array(kids)) = n.get("Plans") {
            for k in kids {
                walk(k, out);
            }
        }
        let node_type = n.get("Node Type").and_then(Value::as_str).unwrap_or("");
        // Rows a node handed up, plus the ones it read and then discarded: for
        // a remote step that sum is what the source actually sent.
        let loops = n.get("Actual Loops").and_then(Value::as_f64).unwrap_or(1.0);
        let dropped = n.get("Rows Removed by Filter").and_then(Value::as_f64).unwrap_or(0.0);
        let rows = n
            .get("Actual Rows")
            .and_then(Value::as_f64)
            .map(|r| ((r + dropped) * loops) as i64);
        let text = |k: &str| n.get(k).and_then(Value::as_str).map(str::to_string);
        if node_type == "Foreign Scan" {
            let remote = text("Remote SQL").or_else(|| text("Remote query")).unwrap_or_default();
            // A single table names itself; a pushed-down join lists its members.
            let (rel, op) = match (text("Schema"), text("Relation Name"), text("Relations")) {
                (Some(s), Some(r), _) => (Some(format!("{s}.{r}")), "scan"),
                (_, _, Some(rels)) => {
                    let first = rels
                        .split(|c: char| c == '(' || c == ')' || c.is_whitespace())
                        .find(|t| t.contains('.'))
                        .map(str::to_string);
                    (first, if rels.starts_with("Aggregate") { "aggregate" } else { "join" })
                }
                _ => (None, "scan"),
            };
            let ds = rel.as_deref().and_then(datasource_of).unwrap_or_else(|| "?".into());
            // A condition the source could not be given is applied here, to
            // rows that have already crossed the wire.
            let detail = match text("Filter") {
                Some(f) => format!("{remote}\n   └ filtered here, after fetching: {f}"),
                None => remote,
            };
            out.push((0, format!("remote:{ds}"), op.to_string(), detail, rows));
        } else if node_type.ends_with("Join") || node_type == "Nested Loop" {
            let cond = text("Hash Cond")
                .or_else(|| text("Merge Cond"))
                .or_else(|| text("Join Filter"))
                .unwrap_or_default();
            // A join on `.nbr` is a hop over stored adjacency: the graph's own
            // edges, joined to whatever the sources returned. One with no
            // condition is the lateral lookup underneath it, which says nothing.
            if cond.contains(".nbr") {
                out.push((0, "local".into(), "stored edge".into(), cond, rows));
            } else if !cond.is_empty() {
                out.push((0, "local".into(), node_type.to_string(), cond, rows));
            }
        }
    }
    if let Some(root) = plan.get(0).and_then(|p| p.get("Plan")) {
        walk(root, &mut out);
    }
    for (i, row) in out.iter_mut().enumerate() {
        row.0 = i as i32 + 1;
    }
    TableIterator::new(out)
}
