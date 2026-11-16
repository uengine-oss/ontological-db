//! The v1 storage layer — the storage `ontological` has always had (P1 K6,
//! `DESIGN.md` 5.2).
//!
//! Everything here knows v1's physical layout: the registries
//! (`og_data.og_node` / `og_edge`), a table per type (`n_<tid>`, `e_<tid>`,
//! `a_<tid>`, history `h_*`) with a column per declared property (`p_*`) and
//! the rest in `__ext`, adjacency segments (`og_data.og_adj`), union views per
//! type, the identifier's bit layout. The modules outside this tree reach it
//! through [`crate::layout::Layout`] (`layout::LayoutV1` here implements it) or,
//! where no seam exists yet, by calling into it by path — every such call is
//! listed in `reports/ontological-v2/P1-K6-split.md` for K7, which turns this
//! tree into the `og-store-v1` crate.
//!
//! What moved here, and from where:
//!
//! | module        | was                                        | what |
//! |---------------|--------------------------------------------|------|
//! | [`layout`]    | `layout/v1.rs`                             | `LayoutV1`: v1's answers to the seam |
//! | [`views`]     | `cypher/views.rs`                          | per-type union views (`v_`, `vp_`, `ve_`, `vep_`) |
//! | [`storage`]   | `storage/mod.rs` (all but five SQL functions) | v1's write paths, role players, TypeQL instances |
//! | [`adjacency`] | `storage/adjacency.rs`                     | adjacency segments (`og_adj`), `og_degree*` |
//! | [`type_tables`] | `catalog/types.rs` (the DDL half)        | a type's table, columns, alias view; `og_create_type` … `og_drop_type` |
//! | [`typeql`]    | `typeql/schema.rs` (`v1_storage`)          | TypeQL types' tables and id spaces |
//! | [`temporal`]  | `temporal/mod.rs`                          | bitemporal history tables (`h_*`) |
//! | [`security`]  | `security/`                                | write grants, stamping, actions, the audit log |
//! | [`privileges`] | `catalog/privileges.rs`                   | `og_grant` / `og_revoke` on v1's schemas |
//! | [`evolution`] | `catalog/evolution.rs`                     | catalog-level schema change (ADR-028) on v1's tables |
//! | [`vector`]    | `vector/mod.rs`                            | embeddings as `vector(N)` columns |
//! | [`history`]   | `agent/mod.rs` (temporal tail)             | `og_history` capture, `og_as_of`, provenance |
//! | [`interop`]   | `interop/mod.rs` (all but `og_cypher_json`) | RLS on type tables, `og_map_table`, materialising |
//! | [`fabric`]    | `fabric/mod.rs` (part of `map_relation`)   | a virtual type's view in v1's row shape |
//! | [`rdf`]       | `adapters/rdf.rs`, `adapters/mod.rs` (scans) | `og_iri`, `og_triple_overflow`, the export's scans |
//! | [`meta`]      | `agent/mod.rs`, `compat/meta.rs` (scans)   | instance counts, observed relationship endpoints |
//!
//! The statements are the ones that ran before the move, text and order: the
//! generated-SQL mold (`tests/seam/`) holds every one of them.

pub mod adjacency;
pub mod evolution;
pub mod fabric;
pub mod history;
pub mod interop;
pub mod layout;
pub mod meta;
pub mod privileges;
pub mod rdf;
pub mod security;
pub mod storage;
pub mod temporal;
pub mod type_tables;
pub mod typeql;
pub mod vector;
pub mod views;
