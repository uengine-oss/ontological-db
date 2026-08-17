//! The `db.*` / `dbms.*` / `apoc.*` procedures, under their Neo4j names.
//!
//! A procedure is planned, not interpreted: each one turns into a relation the
//! compiler puts in `FROM`, so `CALL … YIELD` joins like anything else and the
//! planner costs it. Nothing is executed row by row from Rust.
//!
//! The registry is closed. An unknown procedure is refused by name — an
//! application calling `apoc.something.exotic` should be told so, not handed an
//! empty result that looks like "no matches".

use crate::catalog::types;
use crate::compat::ddl;
use pgrx::prelude::*;

/// One yielded column: the SQL that produces it, and whether it is a graph
/// element (jsonb carrying `_id`/`_type`) or a plain scalar.
pub struct Yielded {
    pub name: &'static str,
    pub sql: String,
    pub is_element: bool,
}

pub struct Plan {
    /// Relation to add to the FROM list, already aliased.
    pub from: String,
    /// Must this relation be LATERAL — does it reference an earlier binding?
    pub lateral: bool,
    pub columns: Vec<Yielded>,
}

/// What the compiler hands over for one argument.
pub enum Arg {
    /// A literal string, known at compile time. Index names must be these.
    Str(String),
    /// The `id` column of a node already bound in this query.
    NodeId(String),
    /// Any other expression, compiled to SQL.
    Sql(String),
}

impl Arg {
    /// The argument as SQL. A literal is *quoted* here — it was kept unquoted
    /// so index names could be read at compile time, and emitting that raw text
    /// into the statement would make it an identifier instead of a value.
    fn sql(&self) -> String {
        match self {
            Arg::Str(s) => crate::cypher::compile::sql_str(s),
            Arg::NodeId(s) | Arg::Sql(s) => s.clone(),
        }
    }

    /// The literal text, for arguments read by the planner rather than passed
    /// through to SQL — index names and APOC's relationship filter.
    fn as_str(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            _ => None,
        }
    }
}

fn elem(name: &'static str, sql: String) -> Yielded {
    Yielded { name, sql, is_element: true }
}

fn scalar(name: &'static str, sql: String) -> Yielded {
    Yielded { name, sql, is_element: false }
}

/// Procedures that exist only so a driver's startup sequence succeeds. They
/// have nothing to wait for — indexes here are built synchronously.
const NO_OPS: &[&str] = &[
    "db.awaitindex",
    "db.awaitindexes",
    "db.index.fulltext.awaiteventuallyconsistentindexrefresh",
    "db.clearquerycaches",
    "db.resamplindex",
];

pub fn plan(
    graph: &str,
    gid: i32,
    name: &str,
    args: &[Arg],
    alias: &str,
) -> Result<Plan, String> {
    let lname = name.to_ascii_lowercase();
    if NO_OPS.contains(&lname.as_str()) {
        return Ok(Plan {
            from: format!("(SELECT) AS {alias}"),
            lateral: false,
            columns: Vec::new(),
        });
    }
    match lname.as_str() {
        "db.index.vector.querynodes" => vector_query(graph, gid, args, alias),
        "db.index.fulltext.querynodes" => fulltext_query(gid, args, alias),
        "apoc.neighbors.tohop" | "apoc.neighbors.tohop.count" => neighbors(args, alias),
        "apoc.meta.schema" => {
            // APOC samples the store to guess a schema; here it is declared, so
            // `sample` bounds only the one thing a declaration cannot supply —
            // the endpoints of a relationship type that never declared roles.
            // See `compat::meta`.
            let sample = match args.first() {
                Some(a) => format!("COALESCE(({})::jsonb ->> 'sample', '1000')::int4", a.sql()),
                None => "1000".to_string(),
            };
            Ok(Plan {
                from: format!(
                    "(SELECT og_apoc_meta_schema({}, {sample})) AS {alias}(value)",
                    crate::cypher::compile::sql_str(graph)
                ),
                lateral: false,
                columns: vec![scalar("value", format!("{alias}.value"))],
            })
        }
        "db.labels" => Ok(Plan {
            from: format!(
                "(SELECT name FROM og_catalog.type WHERE graph_id = {gid} AND kind = 'e') \
                 AS {alias}(label)"
            ),
            lateral: false,
            columns: vec![scalar("label", format!("{alias}.label"))],
        }),
        "db.relationshiptypes" => Ok(Plan {
            from: format!(
                "(SELECT name FROM og_catalog.type WHERE graph_id = {gid} AND kind = 'r') \
                 AS {alias}(t)"
            ),
            lateral: false,
            columns: vec![scalar("relationshipType", format!("{alias}.t"))],
        }),
        "db.propertykeys" => Ok(Plan {
            from: format!(
                "(SELECT DISTINCT p.name FROM og_catalog.property p \
                   JOIN og_catalog.type t ON t.type_id = p.type_id \
                  WHERE t.graph_id = {gid}) AS {alias}(k)"
            ),
            lateral: false,
            columns: vec![scalar("propertyKey", format!("{alias}.k"))],
        }),
        "dbms.components" => Ok(Plan {
            from: format!(
                "(SELECT 'Ontological'::text, ARRAY[ontological_version()]::text[], \
                  'community'::text) AS {alias}(name, versions, edition)"
            ),
            lateral: false,
            columns: vec![
                scalar("name", format!("{alias}.name")),
                scalar("versions", format!("to_jsonb({alias}.versions)")),
                scalar("edition", format!("{alias}.edition")),
            ],
        }),
        other => Err(format!(
            "procedure '{other}' is not available. supported: \
             db.index.vector.queryNodes, db.index.fulltext.queryNodes, \
             apoc.meta.schema, apoc.neighbors.tohop, db.labels, db.relationshipTypes, \
             db.propertyKeys, dbms.components"
        )),
    }
}

/// `db.index.vector.queryNodes(indexName, k, vector) YIELD node, score`
fn vector_query(graph: &str, gid: i32, args: &[Arg], alias: &str) -> Result<Plan, String> {
    if args.len() != 3 {
        return Err("db.index.vector.queryNodes(indexName, k, vector) takes three arguments".into());
    }
    let index = args[0].as_str().ok_or(
        "db.index.vector.queryNodes needs the index name as a literal string — it is resolved \
         when the query is compiled, so a parameter cannot name it",
    )?;
    let entry = ddl::lookup(gid, index).ok_or_else(|| missing_index(gid, index))?;
    if entry.kind != "vector" {
        return Err(format!("index '{index}' is a {} index, not a vector index", entry.kind));
    }
    let prop = entry
        .props
        .first()
        .ok_or_else(|| format!("vector index '{index}' has no property"))?;

    let k = args[1].sql();
    // The query vector arrives as whatever the expression produced: a parameter
    // is a jsonb array, `genai.vector.encode` is a float8[], and a literal is
    // already text. Their text forms differ only in the brackets PostgreSQL
    // puts around an array, and pgvector wants the square ones — so translating
    // accepts all three without having to know which one this is.
    let vec = format!("translate(({})::text, '{{}}', '[]')", args[2].sql());
    Ok(Plan {
        from: format!(
            "og_vector_search({}, {}, {}, {vec}, ({k})::int4) AS {alias}(id, score, entity)",
            crate::cypher::compile::sql_str(graph),
            crate::cypher::compile::sql_str(&entry.type_name),
            crate::cypher::compile::sql_str(prop),
        ),
        lateral: false,
        columns: vec![
            elem("node", format!("{alias}.entity")),
            scalar("score", format!("{alias}.score")),
        ],
    })
}

/// `db.index.fulltext.queryNodes(indexName, query) YIELD node, score`
///
/// Ranked with `ts_rank` over the same `tsvector` the index was built on. See
/// `ddl::fulltext_expr` for why this is a documented difference from Neo4j
/// rather than an equivalence.
fn fulltext_query(gid: i32, args: &[Arg], alias: &str) -> Result<Plan, String> {
    if args.len() < 2 {
        return Err("db.index.fulltext.queryNodes(indexName, query) takes two arguments".into());
    }
    let index = args[0].as_str().ok_or(
        "db.index.fulltext.queryNodes needs the index name as a literal string",
    )?;
    let entry = ddl::lookup(gid, index).ok_or_else(|| missing_index(gid, index))?;
    if entry.kind != "fulltext" {
        return Err(format!("index '{index}' is a {} index, not a full-text index", entry.kind));
    }
    let tid = types::try_type_id(gid, &entry.type_name)
        .ok_or_else(|| format!("index '{index}' refers to a type that no longer exists"))?;

    let ts = ddl::fulltext_expr(&entry.props);
    let q = args[1].sql();
    // The view carries every subtype's rows with the properties as columns, so
    // one scan covers the hierarchy the index was declared on.
    let view = crate::cypher::views::ensure_view(tid, false);
    Ok(Plan {
        from: format!(
            "(SELECT src.id, ts_rank({ts}, websearch_to_tsquery('simple', ({q})::text)) AS score \
               FROM {view} src \
              WHERE {ts} @@ websearch_to_tsquery('simple', ({q})::text) \
              ORDER BY score DESC) AS {alias}(id, score)"
        ),
        lateral: false,
        columns: vec![
            elem("node", format!("og_node_json({alias}.id)")),
            scalar("score", format!("{alias}.score")),
        ],
    })
}

/// `apoc.neighbors.tohop(node, relFilter, distance) YIELD node`
fn neighbors(args: &[Arg], alias: &str) -> Result<Plan, String> {
    if args.is_empty() {
        return Err("apoc.neighbors.tohop(node, relFilter, distance) needs a start node".into());
    }
    let Arg::NodeId(src) = &args[0] else {
        return Err("apoc.neighbors.tohop expects a node bound by an earlier MATCH".into());
    };
    // APOC's relationship filter is a small language; the part that changes the
    // walk is its direction marker. A filter naming types is accepted and its
    // type names honoured.
    let filter = args.get(1).and_then(Arg::as_str).unwrap_or("");
    let dir = if filter.starts_with('<') {
        'i'
    } else if filter.contains('>') {
        'o'
    } else {
        'b'
    };
    let hops = args.get(2).map(Arg::sql).unwrap_or_else(|| "1".to_string());
    Ok(Plan {
        from: format!(
            "og_vlp({src}, NULL::int4[], '{dir}'::\"char\", 1, ({hops})::int) \
             AS {alias}(node, depth, path)"
        ),
        lateral: true,
        columns: vec![
            elem("node", format!("og_node_json({alias}.node)")),
            scalar("depth", format!("{alias}.depth")),
        ],
    })
}

fn missing_index(gid: i32, index: &str) -> String {
    let known: Vec<String> = Spi::connect(|client| {
        client
            .select(
                "SELECT name FROM og_catalog.compat_index WHERE graph_id = $1",
                None,
                &[gid.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    });
    if known.is_empty() {
        format!("there is no index named '{index}'; none have been created in this graph")
    } else {
        format!("there is no index named '{index}'. known indexes: {}", known.join(", "))
    }
}
