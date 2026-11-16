//! `LayoutV1` — the storage layout `ontological` has always had: a registry
//! (`og_data.og_node` / `og_edge`), a table per type (`og_data.n_<tid>`,
//! `e_<tid>`, `a_<tid>`, history `h_*`) with a column per declared property
//! (`p_*`) and the rest in `__ext`, adjacency segments (`og_data.og_adj`), union
//! views per type (`v_` / `vp_` / `ve_` / `vep_`).
//!
//! **An empty skeleton (K1).** Every method is a `todo!` naming the card that
//! moves the v1 code here: K2 the Cypher read compiler (`cypher/compile.rs`,
//! `cypher/views.rs`), K3 the write path (`cypher/mod.rs`, `storage/`), K4
//! TypeQL (`typeql/compile.rs`, `write.rs`, `schema.rs`), K5 traversal
//! (`storage/traverse.rs`). What each must produce is on the trait
//! ([`super::Layout`]): v1's text today, byte for byte. Nothing calls this yet,
//! so none of these panics can be reached.

use super::{Aliases, Dir, ElementId, GraphId, Kind, LabelSet, Layout, PathSem, PropRead, SqlExpr, SqlFrag, TimeScope, TypeId};
use serde_json::Value;

/// The v1 layout. A unit struct: everything it needs is in the catalog.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutV1;

impl Layout for LayoutV1 {
    fn node_scan(&self, g: GraphId, labels: &LabelSet, _at: TimeScope) -> SqlFrag {
        match labels {
            // The union view reads a temporal type's history at the session's
            // instant itself, so the instant does not change the name.
            LabelSet::Under(t) => SqlFrag::new(crate::cypher::views::ensure_view(*t, false)),
            // The registry is shared by every graph — a node's graph is its
            // type's — so it is narrowed to `g`'s types, looked up when the
            // statement runs (a type created meanwhile must not be missed by a
            // cached plan). It holds only the present; the compiler guards an
            // instant (`og_require_current`).
            LabelSet::Any => SqlFrag::new(graph_registry(g)),
            LabelSet::Exactly(_) => {
                unimplemented!("LayoutV1::node_scan: v1 compiles no scan of an exact type set (TypeQL filters the registry)")
            }
        }
    }

    fn edge_scan(&self, _g: GraphId, types: &LabelSet, _at: TimeScope) -> SqlFrag {
        match types {
            LabelSet::Under(t) => SqlFrag::new(crate::cypher::views::ensure_view(*t, true)),
            _ => unimplemented!("LayoutV1::edge_scan: v1 scans relationships of one type only (others are read by id)"),
        }
    }

    fn registry(&self, _kind: Kind) -> SqlFrag {
        todo!("K4: og_data.og_node / og_data.og_edge")
    }

    fn role_players(&self) -> SqlFrag {
        todo!("K4: og_data.og_role_player")
    }

    fn expand(
        &self,
        start: &SqlExpr,
        types: Option<&[TypeId]>,
        dir: Dir,
        temporal: &[TypeId],
        inner: &str,
        names: &mut dyn Aliases,
    ) -> SqlFrag {
        let type_pred = match types {
            Some(ids) => format!(" AND {inner}.etype = ANY({})", int4_array(ids)),
            None => String::new(),
        };
        SqlFrag::new(stored_hop_rows(start, inner, dir, &type_pred, temporal, names))
    }

    fn expand_except(
        &self,
        start: &SqlExpr,
        except: &[TypeId],
        dir: Dir,
        temporal: &[TypeId],
        inner: &str,
        names: &mut dyn Aliases,
    ) -> SqlFrag {
        let type_pred = format!(" AND {inner}.etype <> ALL({})", int4_array(except));
        SqlFrag::new(stored_hop_rows(start, inner, dir, &type_pred, temporal, names))
    }

    fn reach(
        &self,
        start: &SqlExpr,
        types: Option<&[TypeId]>,
        dir: Dir,
        min: u32,
        max: u32,
        sem: PathSem,
    ) -> SqlFrag {
        let f = match sem {
            PathSem::Trails => "og_vlp",
            PathSem::Reach => "og_reach",
        };
        let types = match types {
            Some(ids) => int4_array(ids),
            None => "NULL::int4[]".to_string(),
        };
        let d = match dir {
            Dir::Out => "'o'",
            Dir::In => "'i'",
            Dir::Both => "'b'",
        };
        SqlFrag::new(format!("{f}({start}, {types}, {d}::\"char\", {min}, {max})"))
    }

    fn walk_rels(&self, walk: &str) -> SqlExpr {
        // `og_vlp`'s `path` is the edge ids in order.
        SqlExpr::new(format!(
            "(CASE WHEN {walk}.path IS NULL THEN NULL ELSE (SELECT coalesce(jsonb_agg(og_edge_json(vx.e) \
             ORDER BY vx.o), '[]'::jsonb) FROM unnest({walk}.path) WITH ORDINALITY AS vx(e, o)) END)"
        ))
    }

    fn walk_nodes(&self, start: &SqlExpr, walk: &str) -> SqlExpr {
        // Each edge's other end, walked from the start — the only reading that
        // is right for undirected hops too.
        SqlExpr::new(format!(
            "(CASE WHEN {walk}.path IS NULL THEN NULL ELSE (WITH RECURSIVE pn(i, node) AS (\
             SELECT 0, {start} UNION ALL \
             SELECT pn.i + 1, CASE WHEN pe.src = pn.node THEN pe.dst ELSE pe.src END \
               FROM pn JOIN og_data.og_edge pe ON pe.id = ({walk}.path)[pn.i + 1] \
              WHERE pn.i < cardinality({walk}.path)) \
             SELECT coalesce(jsonb_agg(og_node_json(pn.node) ORDER BY pn.i), '[]'::jsonb) \
               FROM pn WHERE pn.i > 0) END)"
        ))
    }

    fn frontier_step(&self, _dir: Dir, _frontier: &str, _types: &str) -> String {
        todo!("K5: storage::traverse og_reach's per-level SELECT")
    }

    fn prop(&self, alias: &str, scanned: Option<TypeId>, key: &str) -> PropRead {
        use crate::cypher::compile::sql_str;
        if key == "_id" {
            return PropRead { sql: SqlExpr::new(format!("{alias}.id")), sql_type: Some("int8".into()) };
        }
        if let Some(t) = scanned {
            // A column of the view, or — over a wide view — a typed read of
            // the packed value (views::Layout).
            if let Some((sql, dt)) = crate::cypher::views::view_layout(t).read(alias, key) {
                return PropRead { sql: SqlExpr::new(sql), sql_type: Some(dt) };
            }
        }
        let sql = match scanned {
            // Typed view: the property is not declared, so it can only live in
            // the extension payload.
            Some(_) => format!("({alias}.__ext->>{})", sql_str(key)),
            // A registry row: resolved through the catalog at run time.
            None => format!("(og_node_json({alias}.id)->>{})", sql_str(key)),
        };
        PropRead { sql: SqlExpr::new(sql), sql_type: None }
    }

    fn prop_json(&self, alias: &str, scanned: Option<TypeId>, key: &str) -> SqlExpr {
        use crate::cypher::compile::sql_str;
        if key == "_id" {
            return SqlExpr::new(format!("to_jsonb({alias}.id)"));
        }
        if let Some(t) = scanned {
            if let Some(sql) = crate::cypher::views::view_layout(t).read_json(alias, key) {
                return SqlExpr::new(sql);
            }
            return SqlExpr::new(format!("({alias}.__ext -> {})", sql_str(key)));
        }
        SqlExpr::new(format!("(og_node_json({alias}.id) -> {})", sql_str(key)))
    }

    fn prop_type(&self, t: TypeId, key: &str) -> Option<String> {
        crate::cypher::views::view_properties(t).get(key).map(|(_, dt)| dt.clone())
    }

    fn element_json(&self, alias: &str, scanned: Option<TypeId>, kind: Kind) -> SqlExpr {
        use crate::cypher::compile::{json_object, sql_str};
        let Some(t) = scanned else {
            return self.element_json_of_id(&self.element_id(alias), kind);
        };
        // Packed properties of a wide view are already in `__ext`.
        let props = crate::cypher::views::view_layout(t).columns;
        let mut pairs = vec![format!("'_id', {alias}.id"), format!("'_type', og_type_name({alias}.type_id)")];
        if kind == Kind::Rel {
            pairs.push(format!("'_src', {alias}.src"));
            pairs.push(format!("'_dst', {alias}.dst"));
        }
        for (name, (col, _)) in &props {
            pairs.push(format!("{}, to_jsonb({alias}.{col})", sql_str(name)));
        }
        // An OPTIONAL MATCH that found nothing leaves the alias all NULL, and
        // the element is then null — not an empty object, which `count(n)`
        // would count and a client would take for a node.
        SqlExpr::new(format!(
            "(CASE WHEN {alias}.id IS NULL THEN NULL ELSE \
             jsonb_strip_nulls({}) || COALESCE({alias}.__ext,'{{}}'::jsonb) END)",
            json_object(&pairs)
        ))
    }

    fn element_json_of_id(&self, id: &SqlExpr, kind: Kind) -> SqlExpr {
        SqlExpr::new(match kind {
            Kind::Node => format!("og_node_json({id})"),
            Kind::Rel => format!("og_edge_json({id})"),
        })
    }

    fn element_id(&self, alias: &str) -> SqlExpr {
        SqlExpr::new(format!("{alias}.id"))
    }

    fn element_type(&self, alias: &str) -> SqlExpr {
        SqlExpr::new(format!("{alias}.type_id"))
    }

    fn label_of(&self, id: &SqlExpr) -> SqlExpr {
        SqlExpr::new(format!("og_id_type({id})"))
    }

    fn type_of_element(&self, _id: ElementId) -> TypeId {
        todo!("K3/K6: id::id_type")
    }

    fn synthetic_rel_id(&self, rel_type: TypeId, src: &SqlExpr, dst: &SqlExpr) -> SqlExpr {
        // Shard 0, the relationship's type, and the pair hashed into the
        // 36-bit local part (`id.rs`).
        SqlExpr::new(format!(
            "og_make_id(0, {rel_type}, hashtextextended({src}::text || ':' || {dst}::text, 0) & {mask})",
            mask = (1_i64 << 36) - 1,
        ))
    }

    fn temporal_types(&self, g: GraphId, kind: Kind, among: Option<&[TypeId]>) -> Vec<TypeId> {
        use crate::catalog::types;
        match (kind, among) {
            // A relationship type is read from adjacency segments only when it
            // is stored as an edge table; of those, the temporal ones.
            (Kind::Rel, Some(ids)) => ids
                .iter()
                .copied()
                .filter(|t| {
                    types::storage_table(*t).is_some_and(|s| s.starts_with("og_data.e_"))
                        && crate::temporal::is_temporal(*t)
                })
                .collect(),
            (Kind::Rel, None) => crate::temporal::temporal_tables(g, 'e'),
            (Kind::Node, None) => crate::temporal::temporal_tables(g, 'n'),
            // Not asked by the compilers.
            (Kind::Node, Some(ids)) => ids.iter().copied().filter(|t| crate::temporal::is_temporal(*t)).collect(),
        }
    }

    fn size_estimate(&self, _g: GraphId) -> Option<(f64, f64)> {
        // The whole database, not one graph: the registries are shared. The
        // statement's text, line break and indentation included, is what it
        // has always been (the generated-SQL mold traces it).
        let est = crate::spiu::two::<f32, f32>(
            concat!(
                "SELECT (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_node'::regclass),\n",
                "                (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_edge'::regclass)",
            ),
            &[],
        );
        match est {
            Ok((Some(n), Some(e))) if n > 0.0 && e > 0.0 => Some((n as f64, e as f64)),
            _ => None,
        }
    }

    fn take_uncacheable(&self) -> bool {
        todo!("K2/K3: views::take_created_view")
    }

    fn create_node(&self, _g: GraphId, _t: TypeId, _type_name: &str, _props: Value) -> ElementId {
        todo!("K3: storage::create_node_inner")
    }

    fn create_edge(
        &self,
        _g: GraphId,
        _t: TypeId,
        _type_name: &str,
        _src: ElementId,
        _dst: ElementId,
        _props: Value,
    ) -> ElementId {
        todo!("K3: storage::create_edge_inner")
    }

    fn set_node_props(&self, _id: ElementId, _props: Value) {
        todo!("K3: storage::set_node_props_inner")
    }

    fn set_edge_props(&self, _id: ElementId, _props: Value) {
        todo!("K3: storage::set_edge_props_inner")
    }

    fn delete_node(&self, _id: ElementId) -> i64 {
        todo!("K3: storage::delete_node_inner")
    }

    fn delete_edge(&self, _id: ElementId) -> i64 {
        todo!("K3: storage::delete_edge_inner")
    }

    fn degree(&self, _id: ElementId) -> i64 {
        todo!("K3: the og_adj sum in cypher::run_write's DELETE")
    }

    fn refuse_virtual_write(&self, _t: TypeId) {
        todo!("K3: storage::refuse_virtual_write")
    }
}

// ---------------------------------------------------------------------- K2: hop helpers

/// The node registry narrowed to graph `g`'s types (`node_scan(Any)`).
fn graph_registry(g: GraphId) -> String {
    format!(
        "(SELECT gn.* FROM og_data.og_node gn WHERE gn.type_id IN \
         (SELECT gt.type_id FROM og_catalog.type gt WHERE gt.graph_id = {g}))"
    )
}

/// The adjacency segments of one hop, unnested to rows `(nbr, eid)`.
fn adjacency_segments(start: &SqlExpr, a: &str, dir: Dir, type_pred: &str) -> String {
    let dir_pred = match dir {
        Dir::Both => format!("{a}.dir IN ('o','i')"),
        Dir::Out => format!("{a}.dir = 'o'::\"char\""),
        Dir::In => format!("{a}.dir = 'i'::\"char\""),
    };
    format!(
        "SELECT u.nbr, u.eid FROM og_data.og_adj {a}, \
         LATERAL unnest({a}.nbr, {a}.eid) AS u(nbr, eid) \
         WHERE {a}.src = {start} AND {dir_pred}{type_pred}"
    )
}

/// `ARRAY[<ids>]::int4[]`.
fn int4_array(ids: &[TypeId]) -> String {
    format!("ARRAY[{}]::int4[]", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","))
}

/// The stored edges of one hop out of the node whose id is `start`, as rows
/// `(nbr, eid)`: the adjacency segments aliased `a`, narrowed by `type_pred`
/// (which names `a`), and the edge relation of each temporal relationship type
/// in `temporal`, read in place of its segments (they know only the present).
/// The `te` aliases come from the compiler's counter, in this order.
fn stored_hop_rows(
    start: &SqlExpr,
    a: &str,
    dir: Dir,
    type_pred: &str,
    temporal: &[TypeId],
    names: &mut dyn Aliases,
) -> String {
    let segments = adjacency_segments(start, a, dir, type_pred);
    if temporal.is_empty() {
        return segments;
    }
    let mut parts = vec![format!("{segments} AND {a}.etype <> ALL({})", int4_array(temporal))];
    for t in temporal {
        let Some(erel) = crate::temporal::edge_relation(*t) else { continue };
        let te = names.fresh("te");
        if matches!(dir, Dir::Out | Dir::Both) {
            parts.push(format!("SELECT {te}.dst, {te}.id FROM {erel} {te} WHERE {te}.src = {start}"));
        }
        if matches!(dir, Dir::In | Dir::Both) {
            let te2 = names.fresh("te");
            parts.push(format!("SELECT {te2}.src, {te2}.id FROM {erel} {te2} WHERE {te2}.dst = {start}"));
        }
    }
    parts.join(" UNION ALL ")
}

#[cfg(test)]
mod k2_tests {
    //! The text v1 builds without the catalog, as `cypher/compile.rs` wrote it
    //! before K2 (the generated-SQL mold checks the rest against a database).
    //! Only what reaches no SPI is called: this test binary has no PostgreSQL
    //! to resolve SPI's symbols against, and macOS refuses to load a binary
    //! that links them.
    use super::*;

    #[test]
    fn a_hop_reads_the_adjacency_segments() {
        let start = LayoutV1.element_id("n1");
        let types = format!(" AND adj2.etype = ANY({})", int4_array(&[3, 5]));
        assert_eq!(
            adjacency_segments(&start, "adj2", Dir::Out, &types),
            "SELECT u.nbr, u.eid FROM og_data.og_adj adj2, LATERAL unnest(adj2.nbr, adj2.eid) AS u(nbr, eid) \
             WHERE adj2.src = n1.id AND adj2.dir = 'o'::\"char\" AND adj2.etype = ANY(ARRAY[3,5]::int4[])"
        );
        assert!(adjacency_segments(&start, "adj2", Dir::Both, "").ends_with("WHERE adj2.src = n1.id AND adj2.dir IN ('o','i')"));
        assert!(adjacency_segments(&start, "adj2", Dir::In, "").ends_with("adj2.dir = 'i'::\"char\""));
        assert_eq!(int4_array(&[]), "ARRAY[]::int4[]");
    }

    #[test]
    fn a_variable_length_hop_is_a_function_call() {
        let l = LayoutV1;
        let start = l.element_id("n1");
        assert_eq!(
            l.reach(&start, Some(&[4]), Dir::Both, 1, 3, PathSem::Trails).as_str(),
            "og_vlp(n1.id, ARRAY[4]::int4[], 'b'::\"char\", 1, 3)"
        );
        assert_eq!(
            l.reach(&start, None, Dir::In, 1, 30, PathSem::Reach).as_str(),
            "og_reach(n1.id, NULL::int4[], 'i'::\"char\", 1, 30)"
        );
        assert!(l.walk_rels("vl4").as_str().starts_with("(CASE WHEN vl4.path IS NULL THEN NULL ELSE (SELECT"));
        assert!(l.walk_nodes(&start, "vl4").as_str().contains("SELECT 0, n1.id UNION ALL"));
    }

    #[test]
    fn ids_types_and_virtual_relationships() {
        let l = LayoutV1;
        assert_eq!(l.element_type("e3").as_str(), "e3.type_id");
        assert_eq!(l.label_of(&SqlExpr::from("u2.eid")).as_str(), "og_id_type(u2.eid)");
        assert_eq!(l.element_json_of_id(&SqlExpr::from("x"), Kind::Node).as_str(), "og_node_json(x)");
        assert_eq!(l.element_json_of_id(&SqlExpr::from("x"), Kind::Rel).as_str(), "og_edge_json(x)");
        assert_eq!(
            l.synthetic_rel_id(12, &l.element_id("n1"), &l.element_id("n2")).as_str(),
            "og_make_id(0, 12, hashtextextended(n1.id::text || ':' || n2.id::text, 0) & 68719476735)"
        );
        assert_eq!(
            graph_registry(5),
            "(SELECT gn.* FROM og_data.og_node gn WHERE gn.type_id IN \
             (SELECT gt.type_id FROM og_catalog.type gt WHERE gt.graph_id = 5))"
        );
    }
}
