//! Agent-native interface — spec 008.
//!
//! The premise: the entity writing Cypher against this database is increasingly
//! a language model, and it fails differently from a human. It invents labels,
//! reverses relationship directions and writes accidental cartesian products —
//! confidently. So the database owes it three things: an accurate machine-readable
//! schema, errors that carry their own correction, and limits that stop a bad
//! query before it stops the server.

use crate::catalog::types;
use pgrx::prelude::*;
use pgrx::JsonB;
use serde_json::{json, Map, Value};

/// Machine-readable schema — spec 008 FR-001..FR-006.
///
/// `token_budget` is honoured by ranking types by instance count and truncating,
/// because a 1,000-type ontology does not fit in a prompt and an agent given a
/// truncated schema it *knows* is truncated behaves better than one given a
/// complete schema it cannot afford to read.
#[pg_extern(stable)]
fn og_schema(graph: &str, token_budget: default!(Option<i32>, "NULL")) -> JsonB {
    let gid = types::graph_id(graph);
    let version = crate::spiu::one::<i64>(
        "SELECT COALESCE(max(version), 0) FROM og_catalog.schema_version WHERE graph_id = $1 OR graph_id IS NULL",
        &[gid.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0);

    let mut entities: Vec<Value> = Vec::new();
    let mut relations: Vec<Value> = Vec::new();

    let rows: Vec<(i32, String, String, bool, i64)> = Spi::connect(|client| {
        client
            .select(
                "SELECT t.type_id, t.name, t.kind::text, t.is_abstract,
                        COALESCE((SELECT count(*) FROM og_data.og_node n WHERE n.type_id = t.type_id), 0)
                      + COALESCE((SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = t.type_id), 0)
                   FROM og_catalog.type t WHERE t.graph_id = $1
                  ORDER BY 5 DESC, t.name",
                None,
                &[gid.into()],
            )
            .expect("schema scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<i32>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<String>(3).ok()??,
                    r.get::<bool>(4).ok()?.unwrap_or(false),
                    r.get::<i64>(5).ok()?.unwrap_or(0),
                ))
            })
            .collect()
    });

    let total = rows.len();
    // ~30 tokens per type description is a deliberate under-estimate: better to
    // return slightly less than the budget than to blow past it.
    let cap = token_budget.map(|b| ((b as usize) / 30).max(8)).unwrap_or(usize::MAX);
    let truncated = total > cap;

    for (tid, name, kind, is_abstract, instances) in rows.into_iter().take(cap) {
        let parents = parent_names(tid);
        let props = property_list(tid);
        if kind == "r" {
            relations.push(json!({
                "name": name,
                "abstract": is_abstract,
                "extends": parents,
                "instances": instances,
                "roles": role_list(tid),
                "properties": props,
            }));
        } else if kind == "e" {
            entities.push(json!({
                "name": name,
                "abstract": is_abstract,
                "extends": parents,
                "instances": instances,
                "properties": props,
            }));
        }
    }

    let mut out = Map::new();
    out.insert("graph".into(), json!(graph));
    out.insert("schema_version".into(), json!(version));
    out.insert("entity_types".into(), Value::Array(entities));
    out.insert("relation_types".into(), Value::Array(relations));
    out.insert(
        "notes".into(),
        json!([
            "A label matches all of its subtypes: MATCH (v:Vehicle) also returns Car and EV.",
            "Relationship direction matters. Check `roles` for which type sits at each end.",
            "Parameters use $name and are passed as the third argument to og_cypher.",
        ]),
    );
    if truncated {
        out.insert(
            "truncated".into(),
            json!({
                "shown": cap.min(total),
                "total": total,
                "ordered_by": "instance count, descending",
                "hint": "call og_schema_for(graph, question) for the types relevant to one question",
            }),
        );
    }
    JsonB(Value::Object(out))
}

fn parent_names(tid: i32) -> Vec<String> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT p.name FROM og_catalog.type p
                   JOIN og_catalog.type_parent tp ON tp.parent_id = p.type_id
                  WHERE tp.type_id = $1 ORDER BY p.name",
                None,
                &[tid.into()],
            )
            .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
            .unwrap_or_default()
    })
}

fn property_list(tid: i32) -> Vec<Value> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT name, data_type, required, is_key FROM og_catalog.property
                  WHERE type_id = $1 ORDER BY name",
                None,
                &[tid.into()],
            )
            .map(|rows| {
                rows.map(|r| {
                    json!({
                        "name": r.get::<String>(1).unwrap(),
                        "type": r.get::<String>(2).unwrap(),
                        "required": r.get::<bool>(3).unwrap().unwrap_or(false),
                        "key": r.get::<bool>(4).unwrap().unwrap_or(false),
                    })
                })
                .collect()
            })
            .unwrap_or_default()
    })
}

fn role_list(tid: i32) -> Vec<Value> {
    Spi::connect(|client| {
        client
            .select(
                "SELECT r.name, pt.name, r.ordinal, r.card_min, r.card_max
                   FROM og_catalog.role r
                   LEFT JOIN og_catalog.type pt ON pt.type_id = r.player_type_id
                  WHERE r.rel_type_id = $1 ORDER BY r.ordinal",
                None,
                &[tid.into()],
            )
            .map(|rows| {
                rows.map(|r| {
                    json!({
                        "name": r.get::<String>(1).unwrap(),
                        "player_type": r.get::<String>(2).unwrap(),
                        "position": match r.get::<i32>(3).unwrap().unwrap_or(0) {
                            0 => "source",
                            1 => "target",
                            _ => "additional",
                        },
                        "min": r.get::<i32>(4).unwrap(),
                        "max": r.get::<i32>(5).unwrap(),
                    })
                })
                .collect()
            })
            .unwrap_or_default()
    })
}

/// Schema subset relevant to a natural-language question — spec 008 FR-004.
///
/// Lexical overlap only. It is not trying to be clever; it is trying to keep the
/// agent's context small and its label names real.
#[pg_extern(stable)]
fn og_schema_for(graph: &str, question: &str) -> JsonB {
    let gid = types::graph_id(graph);
    let words: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_ascii_lowercase())
        .collect();

    let all: Vec<(i32, String, String)> = Spi::connect(|client| {
        client
            .select(
                "SELECT type_id, name, kind::text FROM og_catalog.type WHERE graph_id = $1",
                None,
                &[gid.into()],
            )
            .expect("type scan failed")
            .filter_map(|r| {
                Some((r.get::<i32>(1).ok()??, r.get::<String>(2).ok()??, r.get::<String>(3).ok()??))
            })
            .collect()
    });

    let mut scored: Vec<(i32, i32, String, String)> = Vec::new();
    for (tid, name, kind) in all {
        let lname = name.to_ascii_lowercase();
        let mut score = 0i32;
        for w in &words {
            if lname.contains(w) || w.contains(&lname) {
                score += 10;
            } else if types::edit_distance(&lname, w) <= 2 {
                score += 4;
            }
        }
        for p in property_list(tid) {
            let pn = p["name"].as_str().unwrap_or("").to_ascii_lowercase();
            if words.iter().any(|w| pn.contains(w) || w.contains(&pn)) {
                score += 3;
            }
        }
        if score > 0 {
            scored.push((score, tid, name, kind));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(12);

    let types_json: Vec<Value> = scored
        .iter()
        .map(|(score, tid, name, kind)| {
            json!({
                "name": name,
                "kind": if kind == "r" { "relation" } else { "entity" },
                "relevance": score,
                "extends": parent_names(*tid),
                "properties": property_list(*tid),
                "roles": if kind == "r" { Value::Array(role_list(*tid)) } else { Value::Null },
            })
        })
        .collect();

    JsonB(json!({
        "graph": graph,
        "question": question,
        "matched_types": types_json,
        "fallback": if types_json.is_empty() {
            json!("no lexical match; call og_schema(graph) for the full schema")
        } else { Value::Null },
    }))
}

/// Structured, correctable error report for a query — spec 008 FR-007..FR-011.
#[pg_extern(stable)]
fn og_explain_error(graph: &str, query: &str) -> JsonB {
    match crate::cypher::parser::parse(query) {
        Err(e) => JsonB(json!({
            "ok": false,
            "code": "CYPHER_PARSE_ERROR",
            "message": e,
            "stage": "parse",
        })),
        Ok(_) => {
            let compiled = std::panic::catch_unwind(|| {
                crate::cypher::compile_for_diagnostics(graph, query)
            });
            match compiled {
                Ok(Ok(_)) => JsonB(json!({ "ok": true })),
                Ok(Err(msg)) => JsonB(json!({
                    "ok": false,
                    "code": classify(&msg),
                    "message": msg,
                    "stage": "compile",
                    "suggestions": suggestions(graph, &msg),
                })),
                Err(_) => JsonB(json!({
                    "ok": false,
                    "code": "INTERNAL",
                    "message": "compilation aborted",
                    "stage": "compile",
                })),
            }
        }
    }
}

fn classify(msg: &str) -> &'static str {
    if msg.contains("unknown label") {
        "UNKNOWN_LABEL"
    } else if msg.contains("not defined") {
        "UNBOUND_VARIABLE"
    } else if msg.contains("unknown function") {
        "UNKNOWN_FUNCTION"
    } else if msg.contains("not supported") {
        "UNSUPPORTED_SYNTAX"
    } else {
        "COMPILE_ERROR"
    }
}

fn suggestions(graph: &str, msg: &str) -> Value {
    if let Some(rest) = msg.split("did you mean:").nth(1) {
        return json!(rest
            .trim()
            .trim_end_matches('.')
            .split(", ")
            .map(|s| s.trim())
            .collect::<Vec<_>>());
    }
    if msg.contains("unknown label") {
        let gid = types::graph_id(graph);
        let names: Vec<String> = Spi::connect(|client| {
            client
                .select(
                    "SELECT name FROM og_catalog.type WHERE graph_id = $1 AND kind = 'e' LIMIT 20",
                    None,
                    &[gid.into()],
                )
                .map(|rows| rows.filter_map(|r| r.get::<String>(1).ok().flatten()).collect())
                .unwrap_or_default()
        });
        return json!(names);
    }
    Value::Null
}

/// Why did this return nothing? — spec 008 FR-010.
///
/// Re-runs the pattern one element at a time and reports the first step whose
/// intermediate result is empty. An agent that knows *where* the match died
/// rewrites the right part of the query.
#[pg_extern]
fn og_diagnose_empty(graph: &str, query: &str) -> JsonB {
    let ast = match crate::cypher::parser::parse(query) {
        Ok(a) => a,
        Err(e) => return JsonB(json!({ "error": e })),
    };
    let steps = crate::cypher::diagnose_pattern(graph, &ast);
    JsonB(json!({ "graph": graph, "steps": steps }))
}

/// Dry-run cost estimate — spec 008 FR-030, FR-031.
#[pg_extern]
fn og_estimate(graph: &str, query: &str) -> JsonB {
    let sql = match crate::cypher::compile_for_diagnostics(graph, query) {
        Ok(s) => s,
        Err(e) => return JsonB(json!({ "error": e })),
    };
    let plan = crate::spiu::one_mut::<JsonB>(
        &format!("EXPLAIN (FORMAT JSON) {sql}"),
        &[JsonB(json!({})).into()],
    )
    .ok()
    .flatten()
    .map(|j| j.0)
    .unwrap_or(Value::Null);

    let root = plan
        .get(0)
        .and_then(|p| p.get("Plan"))
        .cloned()
        .unwrap_or(Value::Null);
    let rows = root.get("Plan Rows").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cost = root.get("Total Cost").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let mut advice: Vec<String> = Vec::new();
    if rows > 1_000_000.0 {
        advice.push(format!(
            "estimated {rows:.0} rows — add a LIMIT or a more selective WHERE clause"
        ));
    }
    if sql.matches("CROSS JOIN").count() > 0 && !sql.contains("LATERAL") {
        advice.push(
            "the pattern contains an unconnected node — connect it with a relationship or it \
             becomes a cartesian product"
                .into(),
        );
    }
    if cost > 1_000_000.0 {
        advice.push("consider an index: og_create_index(graph, type, property)".into());
    }

    JsonB(json!({
        "estimated_rows": rows,
        "estimated_cost": cost,
        "sql": sql,
        "advice": advice,
        "would_run": advice.is_empty(),
    }))
}

// --------------------------------------------------------------------------
// Guardrails — spec 008 FR-024..FR-029
// --------------------------------------------------------------------------

/// Declare resource limits for a role. Applied by `og_apply_role`.
#[pg_extern]
fn og_create_role(name: &str, limits: JsonB) {
    Spi::run_with_args(
        "INSERT INTO og_catalog.agent_role (name, limits) VALUES ($1, $2)
         ON CONFLICT (name) DO UPDATE SET limits = EXCLUDED.limits",
        &[name.into(), limits.into()],
    )
    .expect("role upsert failed");
}

/// Apply a role's limits to the current session.
#[pg_extern]
fn og_apply_role(name: &str) -> JsonB {
    let limits = crate::spiu::one::<JsonB>(
        "SELECT limits FROM og_catalog.agent_role WHERE name = $1",
        &[name.into()],
    )
    .ok()
    .flatten()
    .map(|j| j.0)
    .unwrap_or_else(|| error!("no agent role named '{name}'"));

    if let Some(ms) = limits.get("statement_timeout_ms").and_then(|v| v.as_i64()) {
        Spi::run(&format!("SET statement_timeout = {ms}")).ok();
    }
    if let Some(mem) = limits.get("work_mem_kb").and_then(|v| v.as_i64()) {
        Spi::run(&format!("SET work_mem = {mem}")).ok();
    }
    if let Some(ro) = limits.get("read_only").and_then(|v| v.as_bool()) {
        if ro {
            Spi::run("SET default_transaction_read_only = on").ok();
        }
    }
    if let Some(rows) = limits.get("max_rows").and_then(|v| v.as_i64()) {
        Spi::run(&format!("SET og.max_rows = {rows}")).ok();
    }
    JsonB(json!({ "role": name, "applied": limits }))
}

// --------------------------------------------------------------------------
// Temporal — spec 008 FR-018..FR-023
// --------------------------------------------------------------------------

/// Turn on history capture for a type. Off by default: it costs writes.
#[pg_extern]
fn og_enable_history(graph: &str, type_name: &str) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    for sub in crate::catalog::labeling::og_subtypes(tid) {
        let Some(table) = types::storage_table(sub) else { continue };
        let trig = format!("og_hist_{sub}");
        Spi::run(&format!(
            "CREATE OR REPLACE TRIGGER {trig}
               AFTER INSERT OR UPDATE OR DELETE ON {table}
               FOR EACH ROW EXECUTE FUNCTION og_capture_history()"
        ))
        .unwrap_or_else(|e| error!("failed to enable history on {table}: {e}"));
    }
    Spi::run_with_args(
        "INSERT INTO og_catalog.setting (key, value) VALUES ($1, 'on')
         ON CONFLICT (key) DO UPDATE SET value = 'on'",
        &[format!("history.{graph}.{type_name}").into()],
    )
    .ok();
}

/// Change history for one entity — spec 008 FR-022.
#[pg_extern(stable)]
fn og_history(id: i64) -> TableIterator<
    'static,
    (
        name!(recorded_at, pgrx::datum::TimestampWithTimeZone),
        name!(op, String),
        name!(payload, JsonB),
    ),
> {
    let rows: Vec<_> = Spi::connect(|client| {
        client
            .select(
                "SELECT recorded_at, op::text, payload FROM og_data.og_history
                  WHERE entity_id = $1 ORDER BY recorded_at DESC",
                None,
                &[id.into()],
            )
            .expect("history scan failed")
            .filter_map(|r| {
                Some((
                    r.get::<pgrx::datum::TimestampWithTimeZone>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<JsonB>(3).ok()?.unwrap_or(JsonB(json!({}))),
                ))
            })
            .collect()
    });
    TableIterator::new(rows)
}

/// State of one entity at a past instant — spec 008 FR-020, FR-021.
#[pg_extern(stable)]
fn og_as_of(id: i64, at: pgrx::datum::TimestampWithTimeZone) -> JsonB {
    let tracked = crate::spiu::one::<bool>(
        "SELECT true FROM og_data.og_history WHERE entity_id = $1 LIMIT 1",
        &[id.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(false);
    if !tracked {
        error!(
            "no history is retained for entity {id}. enable it with \
             og_enable_history(graph, type) — returning the current value instead would be a lie"
        );
    }
    crate::spiu::one::<JsonB>(
        "SELECT payload FROM og_data.og_history
          WHERE entity_id = $1 AND recorded_at <= $2
          ORDER BY recorded_at DESC LIMIT 1",
        &[id.into(), at.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(JsonB(Value::Null))
}

/// Attach provenance metadata to an entity — spec 008 FR-015.
#[pg_extern]
fn og_set_source(
    entity_id: i64,
    source: &str,
    confidence: default!(Option<f32>, "NULL"),
    author: default!(Option<&str>, "NULL"),
) {
    Spi::run_with_args(
        "INSERT INTO og_data.og_source (entity_id, source, confidence, author)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (entity_id) DO UPDATE
            SET source = EXCLUDED.source, confidence = EXCLUDED.confidence,
                author = EXCLUDED.author, ingested_at = now()",
        &[entity_id.into(), source.into(), confidence.into(), author.into()],
    )
    .expect("provenance write failed");
}
