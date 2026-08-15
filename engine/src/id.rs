//! Node & edge identifier encoding — spec 001 FR-004.
//!
//! ```text
//!  bit 63        54           36                              0
//!  +---+----------+------------+-------------------------------+
//!  | 0 | shard: 9 | type_id:18 |          local_id: 36         |
//!  +---+----------+------------+-------------------------------+
//! ```
//!
//! Everything a traversal touches is a fixed-width integer: the type of a node
//! is a shift-and-mask, not a catalog join, and the shard bits are reserved up
//! front so spec 007 can distribute without rewriting identifiers.

use pgrx::prelude::*;

pub const LOCAL_BITS: u32 = 36;
pub const TYPE_BITS: u32 = 18;
pub const SHARD_BITS: u32 = 9;

pub const TYPE_SHIFT: u32 = LOCAL_BITS;
pub const SHARD_SHIFT: u32 = LOCAL_BITS + TYPE_BITS;

pub const LOCAL_MASK: i64 = (1i64 << LOCAL_BITS) - 1;
pub const TYPE_MASK: i64 = (1i64 << TYPE_BITS) - 1;
pub const SHARD_MASK: i64 = (1i64 << SHARD_BITS) - 1;

pub const MAX_TYPE_ID: i32 = TYPE_MASK as i32;
pub const MAX_LOCAL_ID: i64 = LOCAL_MASK;
pub const MAX_SHARD_ID: i32 = SHARD_MASK as i32;

/// Compose an identifier. Panics (→ `ereport(ERROR)` via pgrx) on overflow so a
/// silently truncated id can never reach storage.
#[inline]
pub fn make_id(shard: i32, type_id: i32, local: i64) -> i64 {
    if !(0..=MAX_SHARD_ID).contains(&shard) {
        error!("shard id {shard} out of range (0..{MAX_SHARD_ID})");
    }
    if !(0..=MAX_TYPE_ID).contains(&type_id) {
        error!("type id {type_id} out of range (0..{MAX_TYPE_ID})");
    }
    if !(0..=MAX_LOCAL_ID).contains(&local) {
        error!("local id {local} exhausted the 36-bit space for this type");
    }
    ((shard as i64) << SHARD_SHIFT) | ((type_id as i64) << TYPE_SHIFT) | local
}

#[inline]
pub fn id_type(id: i64) -> i32 {
    ((id >> TYPE_SHIFT) & TYPE_MASK) as i32
}

#[inline]
pub fn id_shard(id: i64) -> i32 {
    ((id >> SHARD_SHIFT) & SHARD_MASK) as i32
}

#[inline]
pub fn id_local(id: i64) -> i64 {
    id & LOCAL_MASK
}

/// Re-home an identifier onto another shard, preserving type and local id
/// (spec 007 FR-008: identifiers stay stable across rebalancing).
#[inline]
pub fn with_shard(id: i64, shard: i32) -> i64 {
    make_id(shard, id_type(id), id_local(id))
}

// --------------------------------------------------------------------------
// SQL surface
// --------------------------------------------------------------------------

#[pg_extern(immutable, parallel_safe, strict)]
fn og_id_type(id: i64) -> i32 {
    id_type(id)
}

#[pg_extern(immutable, parallel_safe, strict)]
fn og_id_shard(id: i64) -> i32 {
    id_shard(id)
}

#[pg_extern(immutable, parallel_safe, strict)]
fn og_id_local(id: i64) -> i64 {
    id_local(id)
}

#[pg_extern(immutable, parallel_safe, strict)]
fn og_make_id(shard: i32, type_id: i32, local: i64) -> i64 {
    make_id(shard, type_id, local)
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn roundtrip() {
        let id = ((3i64) << SHARD_SHIFT) | ((4711i64) << TYPE_SHIFT) | 123456789;
        assert_eq!(id_shard(id), 3);
        assert_eq!(id_type(id), 4711);
        assert_eq!(id_local(id), 123456789);
        assert!(id > 0, "identifiers must stay positive");
    }

    #[test]
    fn layout_is_non_overlapping() {
        assert_eq!(LOCAL_BITS + TYPE_BITS + SHARD_BITS, 63);
        assert_eq!(TYPE_MASK << TYPE_SHIFT & LOCAL_MASK, 0);
        assert_eq!(SHARD_MASK << SHARD_SHIFT & (TYPE_MASK << TYPE_SHIFT), 0);
    }
}
