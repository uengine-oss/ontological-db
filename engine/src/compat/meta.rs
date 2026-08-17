//! `apoc.meta.schema()` — the ontology, described in APOC's vocabulary.
//!
//! Every Neo4j tool that wants to know what is in a database asks APOC for it,
//! because Neo4j has no schema of its own to ask: APOC samples the store and
//! reports what it found. Here the schema is declared, so nothing is sampled —
//! the answer is read off `og_catalog` and is exact rather than statistical.
//! That is a difference in kind, and `sample` is accepted and ignored rather
//! than pretended to matter.
//!
//! The shape below is APOC's, not ours. It exists so that a client written
//! against Neo4j — the official `mcp-neo4j-cypher` server among them — reads
//! this database without knowing it is not Neo4j.

use crate::catalog::types;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Map, Value};

/// PostgreSQL column types, named the way APOC names Cypher values.
///
/// A `vector` has no Cypher equivalent — APOC would call it `LIST` and a model
/// reading that would try to compare it with `=`. It is reported under its own
/// name so the difference is visible rather than plausible.
fn apoc_type(data_type: &str) -> &'static str {
    let base = data_type.split(['(', '[']).next().unwrap_or(data_type).trim();
    match base {
        "text" | "uuid" | "string" => "STRING",
        "int2" | "int4" | "int8" | "int" | "long" => "INTEGER",
        "float4" | "float8" | "numeric" | "float" => "FLOAT",
        "bool" => "BOOLEAN",
        "timestamptz" | "timestamp" | "datetime" => "DATE_TIME",
        "date" => "DATE",
        "jsonb" | "json" => "MAP",
        "vector" => "VECTOR",
        _ if data_type.ends_with("[]") => "LIST",
        _ => "STRING",
    }
}

struct Prop {
    name: String,
    data_type: String,
    required: bool,
    is_key: bool,
    indexed: bool,
}

/// Properties of one type, with `indexed` answered by the catalog PostgreSQL
/// actually keeps — a declared index that failed to build would otherwise be
/// reported as present.
fn properties(tid: i32) -> Vec<Prop> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT p.name, p.data_type, p.required, p.is_key,
                        EXISTS (SELECT 1
                                  FROM pg_index i
                                  JOIN pg_attribute a
                                    ON a.attrelid = i.indrelid
                                   AND a.attnum = ANY (i.indkey)
                                 WHERE i.indrelid = t.storage_table::regclass
                                   AND a.attname = p.column_name)
                   FROM og_catalog.property p
                   JOIN og_catalog.type t ON t.type_id = p.type_id
                  WHERE p.type_id = $1 AND t.storage_table IS NOT NULL
                  ORDER BY p.name",
                None,
                &[tid.into()],
            )
            .map(|rows| {
                rows.filter_map(|r| {
                    Some(Prop {
                        name: r.get::<String>(1).ok()??,
                        data_type: r.get::<String>(2).ok()??,
                        required: r.get::<bool>(3).ok()?.unwrap_or(false),
                        is_key: r.get::<bool>(4).ok()?.unwrap_or(false),
                        indexed: r.get::<bool>(5).ok()?.unwrap_or(false),
                    })
                })
                .collect()
            })
            .unwrap_or_default()
    })
}

fn property_map(props: &[Prop]) -> Value {
    let mut out = Map::new();
    for p in props {
        out.insert(
            p.name.clone(),
            json!({
                "type": apoc_type(&p.data_type),
                "indexed": p.indexed,
                "unique": p.is_key,
                "existence": p.required,
            }),
        );
    }
    Value::Object(out)
}

/// A relation type's endpoints, by role ordinal: 0 is the source, 1 the target.
///
/// A relation with more than two roles has no APOC equivalent — APOC describes
/// binary relationships only — so the extra roles are reported on the type's own
/// entry and the pair below is the one a Cypher arrow can express.
fn declared_endpoints(tid: i32) -> Option<(i32, i32)> {
    let pair: Vec<i32> = Spi::connect(|client| {
        client
            .select(
                "SELECT r.player_type_id FROM og_catalog.role r
                  WHERE r.rel_type_id = $1 AND r.ordinal IN (0, 1)
                    AND r.player_type_id IS NOT NULL
                  ORDER BY r.ordinal",
                None,
                &[tid.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<i32>(1).ok().flatten()).collect())
            .unwrap_or_default()
    });
    match pair.as_slice() {
        [src, dst] => Some((*src, *dst)),
        _ => None,
    }
}

/// The endpoints a relation type is *observed* to connect, for types that never
/// declared roles.
///
/// Writing `(a)-[:LINKS]->(b)` with no prior declaration is how Neo4j creates a
/// relationship type, and it is accepted here — so a graph loaded the Neo4j way
/// has relationship types with no roles, and nothing to read a direction off.
/// Rather than report no direction at all, the pairs are read from the edges
/// themselves. This is precisely what APOC does, sampling included, and it
/// carries APOC's caveat with it: what is reported is what the sample contained,
/// not a constraint the database will enforce.
///
/// The type of a node is a bit field of its id, so this needs no join to the
/// node table — and `og_edge_type_idx` leads with `type_id`, so the sample is a
/// bounded index scan rather than a scan of every edge.
fn observed_endpoints(tid: i32, sample: i32) -> Vec<(i32, i32)> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT DISTINCT og_id_type(e.src), og_id_type(e.dst)
                   FROM (SELECT src, dst FROM og_data.og_edge
                          WHERE type_id = $1 ORDER BY id LIMIT $2) e",
                None,
                &[tid.into(), (sample.max(1) as i64).into()],
            )
            .map(|rows| {
                rows.filter_map(|r| Some((r.get::<i32>(1).ok()??, r.get::<i32>(2).ok()??)))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Subtypes answer a label too, so a relationship declared on a supertype is
/// reachable from every descendant. APOC has no inheritance to express this
/// with, so the edge is listed on each concrete label that can actually match.
///
/// `expand` is false for endpoints read from data: those are already the
/// concrete types the edges were found on, and widening them to a hierarchy
/// would claim a reach the sample never showed.
fn label_names(tid: i32, expand: bool) -> Vec<String> {
    let sql = if expand {
        "SELECT name FROM og_catalog.type WHERE type_id = ANY (og_subtypes($1))"
    } else {
        "SELECT name FROM og_catalog.type WHERE type_id = $1"
    };
    Spi::connect(|client| {
        client
            .select(sql, None, &[tid.into()])
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    })
}

/// `apoc.meta.schema()` over one graph.
///
/// `sample` bounds only the fallback in `observed_endpoints` — everything else
/// is read from the catalog, where there is nothing to sample.
#[pg_extern(stable)]
fn og_apoc_meta_schema(graph: &str, sample: default!(i32, 1000)) -> JsonB {
    let gid = types::graph_id(graph);

    let rows: Vec<(i32, String, String, i64)> = Spi::connect(|client| {
        client
            .select(
                "SELECT t.type_id, t.name, t.kind::text,
                        COALESCE((SELECT count(*) FROM og_data.og_node n WHERE n.type_id = t.type_id), 0)
                      + COALESCE((SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = t.type_id), 0)
                   FROM og_catalog.type t
                  WHERE t.graph_id = $1 AND t.kind IN ('e', 'r')
                  ORDER BY t.name",
                None,
                &[gid.into()],
            )
            .expect("meta scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<i32>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                    r.get::<i64>(4).ok()?.unwrap_or(0),
                ))
            })
            .collect()
    });

    // Nodes first: relationships are attached to them in a second pass, which
    // needs every label to already exist as an entry.
    let mut out: Map<String, Value> = Map::new();
    for (tid, name, kind, count) in &rows {
        if kind != "e" {
            continue;
        }
        out.insert(
            name.clone(),
            json!({
                "type": "node",
                "count": count,
                "labels": [],
                "properties": property_map(&properties(*tid)),
                "relationships": Map::new(),
            }),
        );
    }

    for (tid, name, kind, count) in &rows {
        if kind != "r" {
            continue;
        }
        let props = property_map(&properties(*tid));
        out.insert(
            name.clone(),
            json!({ "type": "relationship", "count": count, "properties": props.clone() }),
        );

        // Declared roles are exact and cover the hierarchy. Without them the
        // pairs come from the data, and stand only for themselves.
        let (pairs, expand) = match declared_endpoints(*tid) {
            Some(pair) => (vec![pair], true),
            None => (observed_endpoints(*tid, sample), false),
        };

        for (src_tid, dst_tid) in pairs {
            let sources = label_names(src_tid, expand);
            let targets = label_names(dst_tid, expand);

            for (labels, others, direction) in
                [(&sources, &targets, "out"), (&targets, &sources, "in")]
            {
                for label in labels {
                    let Some(Value::Object(entry)) = out.get_mut(label) else { continue };
                    let Some(Value::Object(rels)) = entry.get_mut("relationships") else {
                        continue;
                    };
                    // A type observed between several label pairs contributes
                    // each target it was seen with, rather than the last one.
                    let slot = rels.entry(name.clone()).or_insert_with(|| {
                        json!({
                            "direction": direction,
                            "count": count,
                            "labels": [],
                            "properties": props.clone(),
                        })
                    });
                    if let Some(Value::Array(labels_out)) = slot.get_mut("labels") {
                        for other in others {
                            let v = Value::String(other.clone());
                            if !labels_out.contains(&v) {
                                labels_out.push(v);
                            }
                        }
                    }
                }
            }
        }
    }

    JsonB(Value::Object(out))
}
