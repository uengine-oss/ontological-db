#!/usr/bin/env node
/**
 * Ontological Studio — backend.
 *
 * Deliberately thin: it holds a connection pool, forwards Cypher to
 * `og_cypher()` and exposes the introspection surface the browser needs. All the
 * intelligence lives in the database, which is the point — anything this server
 * can do, a psql session can do too.
 */
'use strict';

const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { Pool } = require('pg');

const PORT = Number(process.env.PORT || 7474);
const WEB_DIR = path.join(__dirname, '..', 'web');
const BENCH_DIR = process.env.OG_BENCH_DIR || path.join(__dirname, '..', '..', 'bench', 'results');

const pool = new Pool({
  host: process.env.PGHOST || 'localhost',
  port: Number(process.env.PGPORT || 28816),
  database: process.env.PGDATABASE || 'og',
  user: process.env.PGUSER || 'dev',
  password: process.env.PGPASSWORD || undefined,
  max: 8,
  idleTimeoutMillis: 30_000,
});

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
};

function json(res, code, body) {
  const payload = JSON.stringify(body);
  res.writeHead(code, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(payload),
  });
  res.end(payload);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let data = '';
    req.on('data', (c) => {
      data += c;
      if (data.length > 4e6) reject(new Error('request too large'));
    });
    req.on('end', () => {
      try {
        resolve(data ? JSON.parse(data) : {});
      } catch (e) {
        reject(new Error('invalid JSON body'));
      }
    });
    req.on('error', reject);
  });
}

/** Postgres errors carry the correction hints the engine produced — keep them. */
function pgError(e) {
  return {
    error: e.message,
    code: e.code || null,
    detail: e.detail || null,
    hint: e.hint || null,
    where: e.where || null,
  };
}

/**
 * Merge the benchmark result files into one payload per scale.
 *
 * The report page renders whatever the harness last wrote — nobody retypes a
 * latency into HTML, so the site cannot drift from the measurements. Runs are
 * merged newest-wins per system, which is how the harness is actually used:
 * TypeDB takes four minutes to load, so it is often measured in its own run
 * against the same seeded graph.
 */
function readBenchmarks() {
  if (!fs.existsSync(BENCH_DIR)) return { scales: [], source: null };

  const files = fs
    .readdirSync(BENCH_DIR)
    .map((f) => ({ f, m: /^bench-(\d+)-(\d{8}T\d{6}Z)\.json$/.exec(f) }))
    .filter((x) => x.m)
    .sort((a, b) => a.m[2].localeCompare(b.m[2]));

  const byScale = new Map();
  for (const { f, m } of files) {
    let r;
    try {
      r = JSON.parse(fs.readFileSync(path.join(BENCH_DIR, f), 'utf8'));
    } catch {
      continue; // a half-written file is not worth failing the page over
    }
    const key = Number(m[1]);
    if (!byScale.has(key)) {
      byScale.set(key, { scale: r.scale, environment: r.environment, systems: {}, answers: {}, runs: {}, generated_at: r.generated_at });
    }
    const acc = byScale.get(key);
    acc.scale = r.scale;
    acc.generated_at = r.generated_at;
    for (const [name, v] of Object.entries(r.systems || {})) {
      if (!v || !v.queries) continue; // skipped or failed systems never overwrite a good run
      acc.systems[name] = v;
      acc.runs[name] = f.replace(/\.json$/, '');
    }
    for (const [q, v] of Object.entries(r.correctness || {})) {
      acc.answers[q] = { ...(acc.answers[q] || {}), ...v.answers };
    }
    if (r.integrity_violations !== undefined) acc.integrity_violations = r.integrity_violations;
  }

  for (const acc of byScale.values()) {
    acc.correctness = Object.fromEntries(
      Object.entries(acc.answers).map(([q, answers]) => {
        const kept = Object.fromEntries(
          Object.entries(answers).filter(([n]) => acc.systems[n])
        );
        const distinct = [...new Set(Object.values(kept))];
        return [q, { answers: kept, agree: distinct.length <= 1, value: distinct[0] ?? null }];
      })
    );
    delete acc.answers;
  }

  return {
    scales: [...byScale.values()].sort((a, b) => a.scale.nodes - b.scale.nodes),
    source: BENCH_DIR,
  };
}

const routes = {
  /** The benchmark report, read straight from bench/results. */
  async 'GET /api/benchmark'(_req, res) {
    try {
      json(res, 200, readBenchmarks());
    } catch (e) {
      json(res, 500, { error: e.message, scales: [] });
    }
  },

  /** Server + extension health, used by the connection banner. */
  async 'GET /api/health'(_req, res) {
    try {
      const r = await pool.query(
        `SELECT ontological_version() AS version,
                current_database() AS database,
                version() AS server`
      );
      const graphs = await pool.query(
        `SELECT name, graph_id FROM og_catalog.graph ORDER BY name`
      );
      json(res, 200, { ok: true, ...r.rows[0], graphs: graphs.rows });
    } catch (e) {
      json(res, 503, { ok: false, ...pgError(e) });
    }
  },

  /** Sidebar contents: types, relationship types, property keys, counts. */
  async 'GET /api/schema'(req, res, url) {
    const graph = url.searchParams.get('graph') || 'default';
    try {
      const [schema, stats] = await Promise.all([
        pool.query('SELECT og_schema($1) AS s', [graph]),
        pool.query('SELECT og_graph_stats($1) AS s', [graph]),
      ]);
      json(res, 200, { schema: schema.rows[0].s, stats: stats.rows[0].s });
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /** Run a Cypher query and return rows plus a graph projection. */
  async 'POST /api/cypher'(req, res) {
    const body = await readBody(req);
    const { graph = 'default', query = '', params = {} } = body;
    if (!query.trim()) return json(res, 400, { error: 'empty query' });

    const started = Date.now();
    const client = await pool.connect();
    try {
      const r = await client.query('SELECT og_cypher($1,$2,$3) AS row', [
        graph,
        query,
        JSON.stringify(params),
      ]);
      const rows = r.rows.map((x) => x.row);
      let plan = null;
      try {
        const p = await client.query('SELECT og_cypher_sql($1,$2) AS sql', [graph, query]);
        plan = p.rows[0].sql;
      } catch {
        /* write queries have no single compiled statement */
      }
      json(res, 200, {
        rows,
        columns: rows.length ? Object.keys(rows[0]) : [],
        elapsed_ms: Date.now() - started,
        compiled_sql: plan,
        graph: projectGraph(rows),
      });
    } catch (e) {
      json(res, 400, { ...pgError(e), elapsed_ms: Date.now() - started });
    } finally {
      client.release();
    }
  },

  /** The compiled SQL and its PostgreSQL plan — the "look inside" panel. */
  async 'POST /api/explain'(req, res) {
    const { graph = 'default', query = '', analyze = false } = await readBody(req);
    try {
      const r = await pool.query('SELECT og_cypher_explain($1,$2,$3) AS e', [
        graph,
        query,
        analyze,
      ]);
      json(res, 200, r.rows[0].e);
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /** Structured diagnosis for a failing or empty query (spec 008). */
  async 'POST /api/diagnose'(req, res) {
    const { graph = 'default', query = '' } = await readBody(req);
    try {
      const [err, empty, est] = await Promise.all([
        pool.query('SELECT og_explain_error($1,$2) AS d', [graph, query]),
        pool
          .query('SELECT og_diagnose_empty($1,$2) AS d', [graph, query])
          .catch(() => ({ rows: [{ d: null }] })),
        pool
          .query('SELECT og_estimate($1,$2) AS d', [graph, query])
          .catch(() => ({ rows: [{ d: null }] })),
      ]);
      json(res, 200, {
        error: err.rows[0].d,
        empty: empty.rows[0].d,
        estimate: est.rows[0].d,
      });
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /** Expand one node's neighbourhood — double-click in the visualiser. */
  async 'POST /api/expand'(req, res) {
    const { id, limit = 50 } = await readBody(req);
    try {
      const r = await pool.query(
        `SELECT e.eid, e.nbr, og_node_json(e.nbr) AS node,
                og_edge_json(e.eid) AS edge
           FROM og_expand($1::int8, NULL, 'o'::"char") e
          LIMIT $2
          UNION ALL
         SELECT e.eid, e.nbr, og_node_json(e.nbr), og_edge_json(e.eid)
           FROM og_expand($1::int8, NULL, 'i'::"char") e
          LIMIT $2`,
        [id, limit]
      );
      const nodes = [];
      const edges = [];
      for (const row of r.rows) {
        if (row.node) nodes.push(row.node);
        if (row.edge) edges.push(row.edge);
      }
      json(res, 200, { nodes, edges });
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /** Audit trail (spec 008 FR-027) — the "recent queries" panel. */
  async 'GET /api/audit'(_req, res) {
    try {
      const r = await pool.query(
        `SELECT audit_id, principal, at, query, lang, rows_out, duration_ms, error_code
           FROM og_data.og_audit ORDER BY at DESC LIMIT 100`
      );
      json(res, 200, r.rows);
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /** Raw SQL escape hatch — this is still PostgreSQL underneath. */
  async 'POST /api/sql'(req, res) {
    const { sql = '' } = await readBody(req);
    try {
      const r = await pool.query(sql);
      json(res, 200, {
        rows: r.rows,
        columns: r.fields ? r.fields.map((f) => f.name) : [],
        rowCount: r.rowCount,
      });
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },
};

/**
 * Turn result rows into nodes + edges for the visualiser.
 *
 * Anything carrying `_id` is an entity; `_src`/`_dst` make it a relationship.
 * Values are scanned recursively so `RETURN p` and `RETURN collect(n)` both draw.
 */
function projectGraph(rows) {
  const nodes = new Map();
  const edges = new Map();

  const visit = (v) => {
    if (v == null) return;
    if (Array.isArray(v)) return v.forEach(visit);
    if (typeof v !== 'object') return;
    if (v._id !== undefined) {
      if (v._src !== undefined && v._dst !== undefined) {
        edges.set(String(v._id), v);
      } else {
        nodes.set(String(v._id), v);
      }
      return;
    }
    Object.values(v).forEach(visit);
  };
  rows.forEach(visit);

  // Keep only edges whose endpoints are actually on screen.
  const kept = [...edges.values()].filter(
    (e) => nodes.has(String(e._src)) && nodes.has(String(e._dst))
  );
  return { nodes: [...nodes.values()], edges: kept };
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  const key = `${req.method} ${url.pathname}`;

  if (routes[key]) {
    try {
      await routes[key](req, res, url);
    } catch (e) {
      json(res, 500, { error: e.message });
    }
    return;
  }

  // static files
  let file = url.pathname === '/' ? '/index.html' : url.pathname;
  const full = path.join(WEB_DIR, path.normalize(file).replace(/^(\.\.[/\\])+/, ''));
  if (!full.startsWith(WEB_DIR) || !fs.existsSync(full)) {
    res.writeHead(404, { 'content-type': 'text/plain' });
    return res.end('not found');
  }
  res.writeHead(200, { 'content-type': MIME[path.extname(full)] || 'application/octet-stream' });
  fs.createReadStream(full).pipe(res);
});

server.listen(PORT, () => {
  process.stdout.write(
    `Ontological Studio on http://localhost:${PORT}  (postgres ${pool.options.host}:${pool.options.port}/${pool.options.database})\n`
  );
});

process.on('SIGINT', () => {
  pool.end().finally(() => process.exit(0));
});
