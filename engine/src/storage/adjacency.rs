//! Adjacency segments — spec 001 FR-002, FR-003, FR-014, FR-020.
//!
//! One row of `og_data.og_adj` holds up to [`CHUNK`] neighbours of a single node
//! for a single relation type in a single direction, as two aligned `int8[]`.
//! Expanding a node therefore reads *one heap tuple*, not `degree` index entries.
//!
//! Apache AGE stores each edge as an independent row and reaches neighbours
//! through a B-tree on the endpoint column: every hop costs `degree` index
//! probes plus `degree` random heap fetches. That is the gap this file closes.

use pgrx::prelude::*;

/// Neighbours per segment. 256 * 8B * 2 arrays = 4 KB, comfortably inside one
/// 8 KB heap page, which is what keeps the read sequential.
pub const CHUNK: i32 = 256;

/// Append one neighbour to the tail segment, allocating a new chunk when the
/// tail is full.
pub fn append(src: i64, etype: i32, dir: char, nbr: i64, eid: i64) {
    let d = dir.to_string();
    let updated = crate::spiu::one_mut::<i32>(
        "UPDATE og_data.og_adj a
            SET nbr = a.nbr || $4::int8, eid = a.eid || $5::int8, n = a.n + 1
          WHERE a.src = $1 AND a.etype = $2 AND a.dir = $3::text::\"char\"
            AND a.seq = (SELECT max(seq) FROM og_data.og_adj
                          WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\")
            AND a.n < $6
          RETURNING a.seq",
        &[src.into(), etype.into(), d.clone().into(), nbr.into(), eid.into(), CHUNK.into()],
    )
    .expect("adjacency append failed");

    if updated.is_none() {
        Spi::run_with_args(
            "INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
             VALUES ($1, $2, $3::text::\"char\",
                     COALESCE((SELECT max(seq) + 1 FROM og_data.og_adj
                                WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\"), 0),
                     1, ARRAY[$4::int8], ARRAY[$5::int8])",
            &[src.into(), etype.into(), d.into(), nbr.into(), eid.into()],
        )
        .expect("adjacency chunk insert failed");
    }
}

/// Remove one edge from a segment. Keeps the two arrays aligned by splicing at
/// the same index.
pub fn remove(src: i64, etype: i32, dir: char, eid: i64) {
    let d = dir.to_string();
    Spi::run_with_args(
        "UPDATE og_data.og_adj a
            SET nbr = a.nbr[1 : i.idx - 1] || a.nbr[i.idx + 1 : array_length(a.nbr, 1)],
                eid = a.eid[1 : i.idx - 1] || a.eid[i.idx + 1 : array_length(a.eid, 1)],
                n   = a.n - 1
           FROM (SELECT src, etype, dir, seq, array_position(eid, $4::int8) AS idx
                   FROM og_data.og_adj
                  WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\"
                    AND $4::int8 = ANY(eid)
                  LIMIT 1) i
          WHERE a.src = i.src AND a.etype = i.etype AND a.dir = i.dir AND a.seq = i.seq",
        &[src.into(), etype.into(), d.clone().into(), eid.into()],
    )
    .expect("adjacency remove failed");

    // Reclaim emptied segments so scans stay dense.
    Spi::run_with_args(
        "DELETE FROM og_data.og_adj
          WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\" AND n = 0",
        &[src.into(), etype.into(), d.into()],
    )
    .ok();
}

/// Degree of a node for a relation type and direction — the statistic the
/// Cypher planner uses to pick an expansion order (spec 001 FR-015).
#[pg_extern(stable, parallel_safe, strict)]
pub fn og_degree(src: i64, etype: i32, dir: &str) -> i64 {
    crate::spiu::one::<i64>(
        "SELECT COALESCE(sum(n), 0)::int8 FROM og_data.og_adj
          WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\"",
        &[src.into(), etype.into(), dir.into()],
    )
    .expect("degree lookup failed")
    .unwrap_or(0)
}

/// Total degree across all relation types in a direction.
#[pg_extern(stable, parallel_safe, strict)]
pub fn og_degree_all(src: i64, dir: &str) -> i64 {
    crate::spiu::one::<i64>(
        "SELECT COALESCE(sum(n), 0)::int8 FROM og_data.og_adj
          WHERE src = $1 AND dir = $2::text::\"char\"",
        &[src.into(), dir.into()],
    )
    .expect("degree lookup failed")
    .unwrap_or(0)
}
