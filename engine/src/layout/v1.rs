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
    fn node_scan(&self, _g: GraphId, _labels: &LabelSet, _at: TimeScope) -> SqlFrag {
        todo!("K2: views::ensure_view(t, false) / Compiler::graph_nodes")
    }

    fn edge_scan(&self, _g: GraphId, _types: &LabelSet, _at: TimeScope) -> SqlFrag {
        todo!("K2: views::ensure_view(t, true)")
    }

    fn registry(&self, _kind: Kind) -> SqlFrag {
        todo!("K4: og_data.og_node / og_data.og_edge")
    }

    fn role_players(&self) -> SqlFrag {
        todo!("K4: og_data.og_role_player")
    }

    fn expand(
        &self,
        _start: &SqlExpr,
        _types: Option<&[TypeId]>,
        _dir: Dir,
        _temporal: &[TypeId],
        _inner: &str,
        _names: &mut dyn Aliases,
    ) -> SqlFrag {
        todo!("K2: Compiler::stored_hop_rows")
    }

    fn reach(
        &self,
        _start: &SqlExpr,
        _types: Option<&[TypeId]>,
        _dir: Dir,
        _min: u32,
        _max: u32,
        _sem: PathSem,
    ) -> SqlFrag {
        todo!("K2/K5: the og_vlp / og_reach call in Compiler::join_rel")
    }

    fn walk_rels(&self, _walk: &str) -> SqlExpr {
        todo!("K2: var_hop_rels")
    }

    fn walk_nodes(&self, _start: &SqlExpr, _walk: &str) -> SqlExpr {
        todo!("K2: var_hop_nodes")
    }

    fn frontier_step(&self, _dir: Dir, _frontier: &str, _types: &str) -> String {
        todo!("K5: storage::traverse og_reach's per-level SELECT")
    }

    fn prop(&self, _alias: &str, _scanned: Option<TypeId>, _key: &str) -> PropRead {
        todo!("K2: Compiler::prop_sql")
    }

    fn prop_json(&self, _alias: &str, _scanned: Option<TypeId>, _key: &str) -> SqlExpr {
        todo!("K2: Compiler::prop_sql_json")
    }

    fn element_json(&self, _alias: &str, _scanned: Option<TypeId>, _kind: Kind) -> SqlExpr {
        todo!("K2: Compiler::node_json / rel_json")
    }

    fn element_json_of_id(&self, _id: &SqlExpr, _kind: Kind) -> SqlExpr {
        todo!("K2: og_node_json / og_edge_json")
    }

    fn element_id(&self, _alias: &str) -> SqlExpr {
        todo!("K2: <alias>.id")
    }

    fn element_type(&self, _alias: &str) -> SqlExpr {
        todo!("K2: <alias>.type_id")
    }

    fn label_of(&self, _id: &SqlExpr) -> SqlExpr {
        todo!("K2/K4: og_id_type")
    }

    fn type_of_element(&self, _id: ElementId) -> TypeId {
        todo!("K3/K6: id::id_type")
    }

    fn synthetic_rel_id(&self, _rel_type: TypeId, _src: &SqlExpr, _dst: &SqlExpr) -> SqlExpr {
        todo!("K2: virtual_rel_id")
    }

    fn temporal_types(&self, _g: GraphId, _kind: Kind, _among: Option<&[TypeId]>) -> Vec<TypeId> {
        todo!("K2: Compiler::temporal_rel_types / guard_untyped_nodes")
    }

    fn size_estimate(&self, _g: GraphId) -> Option<(f64, f64)> {
        todo!("K2: prefer_reachability's reltuples lookup")
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
