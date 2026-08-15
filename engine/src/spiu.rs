//! SPI helpers.
//!
//! `Spi::get_one_with_args` raises `InvalidPosition` when a query returns no
//! rows, which conflates "nothing matched" with "something broke". Every lookup
//! in this extension needs to tell those apart — a missing type is a normal,
//! reportable condition (spec 008 FR-008), not an internal error.

use pgrx::datum::DatumWithOid;
use pgrx::prelude::*;
use pgrx::spi;

type R<T> = Result<Option<T>, spi::Error>;

/// Read-only single-value lookup. Returns `Ok(None)` for an empty result.
pub fn one<'a, T: FromDatum + IntoDatum>(sql: &str, args: &[DatumWithOid<'a>]) -> R<T> {
    Spi::connect(|client| {
        let mut table = client.select(sql, Some(1), args)?;
        match table.next() {
            Some(row) => row.get::<T>(1),
            None => Ok(None),
        }
    })
}

/// Read-only two-value lookup.
pub fn two<'a, A: FromDatum + IntoDatum, B: FromDatum + IntoDatum>(
    sql: &str,
    args: &[DatumWithOid<'a>],
) -> Result<(Option<A>, Option<B>), spi::Error> {
    Spi::connect(|client| {
        let mut table = client.select(sql, Some(1), args)?;
        match table.next() {
            Some(row) => Ok((row.get::<A>(1)?, row.get::<B>(2)?)),
            None => Ok((None, None)),
        }
    })
}

/// Read-write single-value lookup, for `INSERT ... RETURNING` and friends.
pub fn one_mut<'a, T: FromDatum + IntoDatum>(sql: &str, args: &[DatumWithOid<'a>]) -> R<T> {
    Spi::connect_mut(|client| {
        let mut table = client.update(sql, Some(1), args)?;
        match table.next() {
            Some(row) => row.get::<T>(1),
            None => Ok(None),
        }
    })
}
