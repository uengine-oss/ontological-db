//! Reachability traversal — the deep-hop path.
//!
//! `og_vlp()` (in `sql/access.sql`) enumerates **trails**: every distinct
//! edge-sequence from the start node, carried in an `int8[]` so a path variable
//! can be bound. That is the right answer for `MATCH p = (a)-[r*1..3]->(b)`
//! and the wrong one for `MATCH (a)-[:K*1..6]->(b) RETURN count(DISTINCT b)`,
//! which is the shape almost every application actually asks. The difference is
//! not a constant factor: trail enumeration produces `degree^k` rows, so on a
//! graph of average degree 20 each extra hop costs 20× the last one and the
//! answer stops being computable somewhere around six hops.
//!
//! Reachability does not need paths. A frontier and a visited set touch every
//! node at most once, so the work is bounded by `|V| + |E|` however deep the
//! question goes. Two implementations of that idea live here:
//!
//! * [`og_reach`] — level-synchronous BFS that reads the adjacency segments
//!   through SPI, one query per level. Ordinary heap reads: MVCC, row-level
//!   security and this transaction's own uncommitted writes all still apply.
//! * [`og_csr_build`] / [`og_csr_reach`] — the pgGraph bet. Compile the
//!   topology once into a backend-local CSR of dense `u32` indices and walk it
//!   with no SPI, no heap and no planner in the loop. Faster, and it gives up
//!   exactly what leaving the heap gives up: the snapshot is frozen at build
//!   time and RLS is never consulted.
//!
//! Both are measured against `og_vlp` by `bench/csr/`.

use pgrx::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Direction letters the adjacency table stores, plus `b` for either.
fn check_dir(dir: &str) -> &str {
    match dir {
        "o" | "i" | "b" => dir,
        _ => error!("direction must be 'o', 'i' or 'b', not '{dir}'"),
    }
}

/// The opposite of a direction — `b` is its own opposite.
fn flip(dir: &str) -> &'static str {
    match dir {
        "o" => "i",
        "i" => "o",
        _ => "b",
    }
}

/// Predicate over `og_adj` for a direction and an optional relation-type list.
///
/// The direction is validated against a three-element set before it reaches
/// here, so the format is not an injection path; the type ids still go in as a
/// bound parameter, because a thousand-element list rewritten per call would be
/// its own cost.
fn adj_where(dir: &str, alias: &str, types_param: &str) -> String {
    let d = match check_dir(dir) {
        "b" => format!("{alias}.dir IN ('o','i')"),
        other => format!("{alias}.dir = '{other}'::\"char\""),
    };
    format!("{d} AND ({types_param}::int4[] IS NULL OR {alias}.etype = ANY({types_param}::int4[]))")
}

// ---------------------------------------------------------------------------
// og_reach — BFS over the heap
// ---------------------------------------------------------------------------

/// Nodes reachable from `src` in `minhop`..`maxhop` hops, each reported once at
/// the depth it was first reached.
///
/// This is `og_vlp` minus the paths: same segments, same visibility rules, but
/// a visited set instead of a trail, so the row count is bounded by the number
/// of reachable nodes rather than by the number of walks.
///
/// The signature deliberately matches `og_vlp`'s up to the path column —
/// `dir` is `"char"`, not `text` — so the Cypher compiler can emit one or the
/// other into the same LATERAL join without reshaping anything around it.
#[pg_extern(stable, parallel_restricted)]
fn og_reach(
    src: i64,
    etypes: Option<Vec<i32>>,
    dir: i8,
    minhop: i32,
    maxhop: i32,
) -> TableIterator<'static, (name!(node, i64), name!(depth, i32))> {
    let dir = match dir as u8 {
        b'o' => "o",
        b'i' => "i",
        b'b' => "b",
        other => error!("direction must be 'o', 'i' or 'b', not '{}'", other as char),
    };
    let sql = format!(
        "SELECT a.nbr FROM og_data.og_adj a WHERE a.src = ANY($1::int8[]) AND {}",
        adj_where(dir, "a", "$2")
    );

    // One SPI connection and one plan for the whole walk, not one per level.
    //
    // This matters far more than it looks. On a graph with a large diameter the
    // frontier can be a single node and the level count can be six figures —
    // a 1,000,000-node chain walked to 100,000 hops does almost no work per
    // level and nothing but levels. Connecting and re-planning inside the loop
    // made that case ten times slower than the same walk written as a plain
    // recursive CTE, which was the measurement that found this.
    let out = Spi::connect(|client| {
        let stmt = client
            .prepare(
                sql.as_str(),
                &[
                    PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID),
                    PgOid::BuiltIn(PgBuiltInOids::INT4ARRAYOID),
                ],
            )
            .unwrap_or_else(|e| error!("adjacency scan could not be planned: {e}"));

        let mut visited: HashSet<i64> = HashSet::with_capacity(1024);
        visited.insert(src);
        let mut frontier = vec![src];
        let mut out: Vec<(i64, i32)> = Vec::new();
        // The start node is visited at depth 0 so the frontier never re-expands
        // it, but a cycle that comes back to it still makes it an answer —
        // `(a)-[*1..k]->(b)` binds `b = a` when one exists. Emitted once, at the
        // depth it returns.
        let mut start_done = minhop <= 0;
        if minhop <= 0 {
            out.push((src, 0));
        }

        for depth in 1..=maxhop {
            if frontier.is_empty() {
                break;
            }
            let segments: Vec<Vec<Option<i64>>> = client
                .select(&stmt, None, &[frontier.clone().into(), etypes.clone().into()])
                .unwrap_or_else(|e| error!("adjacency scan failed: {e}"))
                .filter_map(|r| r.get::<Vec<Option<i64>>>(1).ok().flatten())
                .collect();

            let mut next = Vec::new();
            for seg in segments {
                for nbr in seg.into_iter().flatten() {
                    if visited.insert(nbr) {
                        next.push(nbr);
                        if depth >= minhop {
                            out.push((nbr, depth));
                        }
                    } else if nbr == src && !start_done && depth >= minhop {
                        start_done = true;
                        out.push((src, depth));
                    }
                }
            }
            frontier = next;
        }
        out
    });

    TableIterator::new(out)
}

// ---------------------------------------------------------------------------
// Backend-local CSR — the pgGraph shape
// ---------------------------------------------------------------------------

/// One direction of a compiled adjacency: `nbrs[offs[i]..offs[i+1]]`.
struct Adj {
    offs: Vec<u32>,
    nbrs: Vec<u32>,
}

impl Adj {
    fn of(&self, i: u32) -> &[u32] {
        &self.nbrs[self.offs[i as usize] as usize..self.offs[i as usize + 1] as usize]
    }
    fn bytes(&self) -> usize {
        self.offs.len() * 4 + self.nbrs.len() * 4
    }
}

/// Compiled topology: dense indices, contiguous neighbour arrays, both ways.
///
/// Node ids are 64-bit and sparse; the CSR stores `u32` positions into a sorted
/// id vector instead, which halves the arrays and keeps a frontier scan inside
/// cache. `ids` is the only thing that turns a position back into an id. The
/// reverse direction is compiled too — `og_adj` already stores it, and a
/// bidirectional shortest path is not correct without it.
struct Csr {
    key: String,
    ids: Vec<i64>,
    fwd: Adj,
    rev: Adj,
}

impl Csr {
    fn index_of(&self, id: i64) -> Option<u32> {
        self.ids.binary_search(&id).ok().map(|p| p as u32)
    }
    fn bytes(&self) -> usize {
        self.ids.len() * 8 + self.fwd.bytes() + self.rev.bytes()
    }
}

thread_local! {
    /// One compiled graph per backend. Rust-heap allocated on purpose: a
    /// PostgreSQL memory context would free it at end of transaction, and the
    /// whole point is that the next statement finds it already built.
    static CSR: RefCell<Option<Csr>> = const { RefCell::new(None) };
}

fn build_key(etypes: &Option<Vec<i32>>, dir: &str) -> String {
    match etypes {
        Some(t) => {
            let mut t = t.clone();
            t.sort_unstable();
            format!("{dir}:{t:?}")
        }
        None => format!("{dir}:*"),
    }
}

/// Turn an edge list of dense positions into CSR. Counting sort, two passes.
fn pack(n: usize, edges: &[(u32, u32)]) -> Adj {
    let mut offs = vec![0u32; n + 1];
    for (s, _) in edges {
        offs[*s as usize + 1] += 1;
    }
    for i in 0..n {
        offs[i + 1] += offs[i];
    }
    let mut nbrs = vec![0u32; edges.len()];
    let mut fill = offs.clone();
    for (s, d) in edges {
        nbrs[fill[*s as usize] as usize] = *d;
        fill[*s as usize] += 1;
    }
    Adj { offs, nbrs }
}

fn compile(etypes: Option<Vec<i32>>, dir: &str) -> Csr {
    // One scan covers both directions: `og_adj` stores an 'i' segment for every
    // 'o' segment, so the reverse CSR costs no extra I/O.
    let sql = format!(
        "SELECT a.src, a.dir::text, a.nbr FROM og_data.og_adj a WHERE {}",
        adj_where("b", "a", "$1")
    );
    let rows: Vec<(i64, String, Vec<Option<i64>>)> = Spi::connect(|client| {
        client
            .select(&sql, None, &[etypes.clone().into()])
            .unwrap_or_else(|e| error!("adjacency scan failed: {e}"))
            .filter_map(|r| {
                Some((
                    r.get::<i64>(1).ok()??,
                    r.get::<String>(2).ok()??,
                    r.get::<Vec<Option<i64>>>(3).ok()??,
                ))
            })
            .collect()
    });

    // Every id that appears on either end becomes a dense position. Sorting
    // once is cheaper than a hash map probed |E| times, and it leaves the
    // neighbour lists in a layout a prefetcher can follow.
    let mut ids: Vec<i64> = Vec::with_capacity(rows.len() * 2);
    for (src, _, seg) in &rows {
        ids.push(*src);
        ids.extend(seg.iter().flatten().copied());
    }
    ids.sort_unstable();
    ids.dedup();
    let pos = |id: i64| ids.binary_search(&id).expect("id present by construction") as u32;

    let want = check_dir(dir);
    let back = flip(want);
    let (mut fwd_e, mut rev_e) = (Vec::new(), Vec::new());
    for (src, d, seg) in &rows {
        let s = pos(*src);
        for nbr in seg.iter().flatten() {
            let t = pos(*nbr);
            if want == "b" || d == want {
                fwd_e.push((s, t));
            }
            if want == "b" || d == back {
                rev_e.push((s, t));
            }
        }
    }

    let n = ids.len();
    Csr { key: build_key(&etypes, want), fwd: pack(n, &fwd_e), rev: pack(n, &rev_e), ids }
}

/// Compile the topology into this backend's memory. Returns what it built.
#[pg_extern]
fn og_csr_build(
    etypes: Option<Vec<i32>>,
    dir: default!(&str, "'o'"),
) -> TableIterator<
    'static,
    (name!(nodes, i64), name!(edges, i64), name!(bytes, i64), name!(build_ms, f64)),
> {
    let t0 = std::time::Instant::now();
    let csr = compile(etypes, dir);
    let row = (
        csr.ids.len() as i64,
        csr.fwd.nbrs.len() as i64,
        csr.bytes() as i64,
        t0.elapsed().as_secs_f64() * 1000.0,
    );
    CSR.with(|c| *c.borrow_mut() = Some(csr));
    TableIterator::once(row)
}

/// Discard this backend's compiled graph.
#[pg_extern]
fn og_csr_drop() {
    CSR.with(|c| *c.borrow_mut() = None);
}

/// What this backend currently holds, if anything.
#[pg_extern]
fn og_csr_stats() -> TableIterator<
    'static,
    (name!(built_for, Option<String>), name!(nodes, i64), name!(edges, i64), name!(bytes, i64)),
> {
    let row = CSR.with(|c| match &*c.borrow() {
        Some(g) => {
            (Some(g.key.clone()), g.ids.len() as i64, g.fwd.nbrs.len() as i64, g.bytes() as i64)
        }
        None => (None, 0, 0, 0),
    });
    TableIterator::once(row)
}

fn with_csr<T>(f: impl FnOnce(&Csr) -> T) -> T {
    CSR.with(|c| {
        let borrow = c.borrow();
        let g = borrow.as_ref().unwrap_or_else(|| {
            error!("no compiled graph in this backend — call og_csr_build() first")
        });
        f(g)
    })
}

/// Set a bit; report whether it was clear before.
#[inline]
fn mark(seen: &mut [u64], i: u32) -> bool {
    let (w, b) = ((i >> 6) as usize, 1u64 << (i & 63));
    let fresh = seen[w] & b == 0;
    seen[w] |= b;
    fresh
}

/// Nodes reachable from `src`, walked entirely inside the compiled array.
///
/// No SPI, no heap, no planner. Build it first with [`og_csr_build`]; the
/// snapshot is whatever was committed and visible then.
#[pg_extern(stable, parallel_restricted)]
fn og_csr_reach(
    src: i64,
    minhop: i32,
    maxhop: i32,
) -> TableIterator<'static, (name!(node, i64), name!(depth, i32))> {
    let out = with_csr(|g| {
        let Some(start) = g.index_of(src) else { return Vec::new() };

        let mut seen = vec![0u64; g.ids.len().div_ceil(64)];
        mark(&mut seen, start);

        let mut out: Vec<(i64, i32)> = Vec::new();
        // See `og_reach`: the start node is an answer too if a cycle returns to it.
        let mut start_done = minhop <= 0;
        if minhop <= 0 {
            out.push((src, 0));
        }
        let (mut frontier, mut next) = (vec![start], Vec::new());
        for depth in 1..=maxhop {
            if frontier.is_empty() {
                break;
            }
            next.clear();
            for &n in &frontier {
                for &m in g.fwd.of(n) {
                    if mark(&mut seen, m) {
                        next.push(m);
                        if depth >= minhop {
                            out.push((g.ids[m as usize], depth));
                        }
                    } else if m == start && !start_done && depth >= minhop {
                        start_done = true;
                        out.push((src, depth));
                    }
                }
            }
            std::mem::swap(&mut frontier, &mut next);
        }
        out
    });
    TableIterator::new(out)
}

/// One level of a bidirectional search: advance `frontier` by a hop, recording
/// distances in `near`, and report the best `near + far` meeting found. The
/// whole level is scanned before the caller may answer, because a meeting found
/// early in a level is not necessarily the shortest one in it.
fn level(
    adj: &Adj,
    frontier: &[u32],
    near: &mut [i32],
    far: &[i32],
    maxhop: i32,
) -> (Vec<u32>, i32) {
    let mut next = Vec::new();
    let mut best = i32::MAX;
    for &n in frontier {
        let dn = near[n as usize];
        if dn >= maxhop {
            continue;
        }
        for &m in adj.of(n) {
            if near[m as usize] < 0 {
                near[m as usize] = dn + 1;
                next.push(m);
            }
            if far[m as usize] >= 0 {
                best = best.min(near[m as usize] + far[m as usize]);
            }
        }
    }
    (next, best)
}

/// Hop count of the shortest path between two nodes, NULL if unreachable
/// within `maxhop`.
///
/// Bidirectional: two frontiers meeting in the middle explore roughly
/// `2·d^(k/2)` nodes instead of `d^k`. The reverse frontier walks the reverse
/// CSR, so this is a directed shortest path when the graph was built directed.
/// A level is always finished before answering — stopping at the first meeting
/// would report a path one hop longer than the shortest often enough to matter.
#[pg_extern(stable, parallel_restricted)]
fn og_csr_hops(src: i64, dst: i64, maxhop: default!(i32, 32)) -> Option<i32> {
    with_csr(|g| {
        let (s, d) = (g.index_of(src)?, g.index_of(dst)?);
        if s == d {
            return Some(0);
        }
        const UNSEEN: i32 = -1;
        let mut ds = vec![UNSEEN; g.ids.len()];
        let mut dd = vec![UNSEEN; g.ids.len()];
        ds[s as usize] = 0;
        dd[d as usize] = 0;
        let (mut fs, mut fd) = (vec![s], vec![d]);

        while !fs.is_empty() && !fd.is_empty() {
            // Expand whichever frontier is smaller — that is what keeps the
            // search balanced on a skewed graph.
            let forward = fs.len() <= fd.len();
            let (next, best) = if forward {
                level(&g.fwd, &fs, &mut ds, &dd, maxhop)
            } else {
                level(&g.rev, &fd, &mut dd, &ds, maxhop)
            };
            if best <= maxhop {
                return Some(best);
            }
            if forward {
                fs = next;
            } else {
                fd = next;
            }
        }
        None
    })
}
