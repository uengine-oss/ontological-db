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
const crypto = require('node:crypto');
const { Pool } = require('pg');

const PORT = Number(process.env.PORT || 7474);

// `server.listen(PORT, cb)` binds every interface, and the startup line said
// "localhost" while doing it — so a laptop on a shared network was serving
// unauthenticated SQL to that network and reporting otherwise. Loopback is the
// default now, and reaching further is something you have to ask for twice.
const HOST = process.env.OG_STUDIO_HOST || '127.0.0.1';
const ALLOW_REMOTE = process.env.OG_STUDIO_ALLOW_REMOTE === '1';
const LOOPBACK = new Set(['127.0.0.1', '::1', 'localhost']);

// The raw-SQL escape hatch is a development convenience that happens to be a
// remote shell into the database. Off unless asked for.
const ALLOW_RAW_SQL = process.env.OG_STUDIO_ALLOW_RAW_SQL === '1';
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

/**
 * Refuse anything that did not come from this page.
 *
 * There is no CORS handler here, so a cross-origin script cannot read a reply —
 * but it can still send one. A form POST with a `text/plain` body is a "simple
 * request": no preflight, and the old `readBody` parsed it as JSON regardless.
 * That is a blind write against the database of whoever visits the wrong page,
 * and binding to loopback does not stop it, because the browser making the
 * request is already inside.
 *
 * Two checks close it. `Sec-Fetch-Site` is set by the browser and cannot be
 * spoofed from script; `Origin` is sent on every cross-origin write. A request
 * carrying neither is not from a browser — curl and psql-style clients keep
 * working, and they were never the ones being tricked.
 */
function sameOrigin(req) {
  const site = req.headers['sec-fetch-site'];
  if (site && site !== 'same-origin' && site !== 'none') return false;
  const origin = req.headers.origin;
  if (origin) {
    try {
      if (new URL(origin).host !== req.headers.host) return false;
    } catch {
      return false;
    }
  }
  return true;
}

/**
 * A body has to be declared as JSON to be read as JSON.
 *
 * This is the other half of the CSRF fix: `application/json` is not a content
 * type a simple request can carry, so requiring it forces a preflight, and
 * there is nothing here to answer one.
 */
function declaresJson(req) {
  const ct = (req.headers['content-type'] || '').split(';')[0].trim().toLowerCase();
  return ct === 'application/json';
}

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


// --------------------------------------------------------- schema assist ---

const ASSIST_SYSTEM = `You turn a request about an ontology graph schema into schema changes.
Reply with one JSON object: {"explanation": "<one or two sentences in the user's language>", "changes": [ ... ]}.
Each change is one of these, exactly:
  {"kind":"create_type","type":"<NewName>","parents":["<Existing>"]}         new entity type
  {"kind":"rename_property","type":"<T>","property":"<p>","new_name":"<q>"}  catalog only
  {"kind":"rename_type","type":"<T>","new_name":"<U>"}                       catalog only
  {"kind":"add_parent","type":"<T>","parent":"<P>"}                          catalog only
  {"kind":"remove_parent","type":"<T>","parent":"<P>"}                       catalog only
  {"kind":"drop_property","type":"<T>","property":"<p>"}
  {"kind":"alter_property_type","type":"<T>","property":"<p>","to":"int|float|string|bool|date|timestamp"}  rewrites rows
  {"kind":"set_required","type":"<T>","property":"<p>","default":<value or omit>}
  {"kind":"add_property","type":"<T>","property":"<p>","data_type":"int|float|string|bool|date|timestamp"}
Rules:
- A property can be renamed, retyped, required or dropped only on the type that DECLARES it (marked "declared" below).
- To insert a type between a parent and a child: create_type under the parent, add_parent child→new, remove_parent child→old parent. Order matters.
- Use existing names exactly. New names are identifiers (letters, digits, underscore).
- If the request is unclear or impossible, return no changes and say why in explanation.
Current schema:
{CATALOG}`;

/** A compact, model-readable catalog: each type, its parents, declared and inherited properties. */
function describeCatalog(snap) {
  const types = new Map((snap.types || []).map((t) => [t.name, t]));
  const ancestors = (name, seen = new Set()) => {
    for (const p of types.get(name)?.parents || []) if (!seen.has(p)) { seen.add(p); ancestors(p, seen); }
    return seen;
  };
  const declared = new Map();
  for (const t of types.values()) {
    const inherited = new Set([...ancestors(t.name)].flatMap((a) => Object.keys(types.get(a)?.properties || {})));
    declared.set(t.name, Object.keys(t.properties || {}).filter((p) => !inherited.has(p)));
  }
  const lines = [...types.values()].map((t) => {
    const props = Object.entries(t.properties || {}).map(([k, v]) =>
      `${k}:${v.type}${declared.get(t.name).includes(k) ? ' (declared)' : ''}`).join(', ');
    return `- ${t.name} [${t.kind === 'r' ? 'relation' : 'entity'}]${t.parents?.length ? ' extends ' + t.parents.join(', ') : ''}${t.temporal ? ' (temporal)' : ''}: ${props || '—'}`;
  });
  return { text: lines.join('\n'), types, declared };
}

const ID = /^[A-Za-z_][A-Za-z0-9_]*$/;
const KINDS = new Set(['create_type', 'rename_property', 'rename_type', 'add_parent', 'remove_parent', 'drop_property',
                       'alter_property_type', 'set_required', 'add_property']);
/** Check proposed changes in order against the catalog (and the changes before them). */
function checkChanges(changes, catalog) {
  const types = new Set(catalog.types.keys());
  const declared = new Map([...catalog.declared].map(([k, v]) => [k, new Set(v)]));
  const parents = new Map([...catalog.types].map(([k, t]) => [k, new Set(t.parents || [])]));
  return changes.map((c) => {
    const problem = (() => {
      if (!c || !KINDS.has(c.kind)) return `unknown kind ${c?.kind}`;
      if (c.kind === 'create_type') {
        if (!ID.test(c.type || '')) return 'new type name is not an identifier';
        if (types.has(c.type)) return `type ${c.type} already exists`;
        for (const p of c.parents || []) if (!types.has(p)) return `parent ${p} does not exist`;
        types.add(c.type); declared.set(c.type, new Set()); parents.set(c.type, new Set(c.parents || []));
        return null;
      }
      if (!types.has(c.type)) return `type ${c.type} does not exist`;
      const decl = declared.get(c.type);
      if (['rename_property', 'drop_property', 'alter_property_type', 'set_required'].includes(c.kind)) {
        if (!decl.has(c.property)) return `${c.property} is not declared on ${c.type}`;
      }
      if (c.kind === 'rename_property') {
        if (!ID.test(c.new_name || '')) return 'new name is not an identifier';
        decl.delete(c.property); decl.add(c.new_name);
      }
      if (c.kind === 'drop_property') decl.delete(c.property);
      if (c.kind === 'add_property') { if (!ID.test(c.property || '')) return 'property name is not an identifier'; decl.add(c.property); }
      if (c.kind === 'rename_type') {
        if (!ID.test(c.new_name || '')) return 'new name is not an identifier';
        types.delete(c.type); types.add(c.new_name); declared.set(c.new_name, decl); parents.set(c.new_name, parents.get(c.type));
      }
      if (c.kind === 'add_parent') {
        if (!types.has(c.parent)) return `parent ${c.parent} does not exist`;
        parents.get(c.type).add(c.parent);
      }
      if (c.kind === 'remove_parent') {
        if (!parents.get(c.type)?.has(c.parent)) return `${c.parent} is not a parent of ${c.type}`;
        parents.get(c.type).delete(c.parent);
      }
      return null;
    })();
    return { change: c, ok: !problem, problem };
  });
}

/** Set og.valid_at / og.system_at on one connection; returns what was set. */
async function applyInstant(client, at = {}) {
  const set = {};
  for (const [key, setting] of [['valid_at', 'og.valid_at'], ['system_at', 'og.system_at']]) {
    if (at && at[key]) {
      await client.query('SELECT set_config($1, $2, false)', [setting, String(at[key])]);
      set[key] = String(at[key]);
    }
  }
  return Object.keys(set).length ? set : null;
}

/**
 * Rows of the changed type (and its subtypes) written since the change was
 * planned. PostgreSQL stamps every row version with the writing transaction
 * (`xmin`), so a catalog-only change leaves this at 0 and a rewrite shows up
 * as the number of rows it touched.
 */
const changeWatch = new Map();
async function rowsWritten(id) {
  const w = changeWatch.get(id);
  if (!w) return null;
  const tables = (await pool.query(
    `WITH RECURSIVE t AS (
       SELECT ty.type_id FROM og_catalog.type ty JOIN og_catalog.graph g USING (graph_id)
        WHERE g.name = $1 AND ty.name = $2
       UNION SELECT p.type_id FROM og_catalog.type_parent p JOIN t ON p.parent_id = t.type_id)
     SELECT storage_table FROM og_catalog.type WHERE type_id IN (SELECT type_id FROM t)
        AND storage_table IS NOT NULL`, [w.graph, w.type])).rows.map((r) => r.storage_table);
  let rows = 0, written = 0;
  for (const tbl of tables) {
    const r = (await pool.query(
      `SELECT count(*)::int8 AS n,
              count(*) FILTER (WHERE xmin::text::int8 >= ($1::int8 % 4294967296))::int8 AS w FROM ${tbl}`,
      [w.xid])).rows[0];
    rows += Number(r.n); written += Number(r.w);
  }
  return { rows, written, tables };
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
    const { graph = 'default', query = '', params = {}, at = {} } = body;
    if (!query.trim()) return json(res, 400, { error: 'empty query' });

    const started = Date.now();
    const client = await pool.connect();
    try {
      // Point-in-time (spec 013): the instant is a session setting the generated
      // type views read, so the Cypher text — and its compiled SQL — never change.
      // Set on this connection only, and reset in `finally` before it returns to
      // the pool.
      const instant = await applyInstant(client, at);
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
        // Same text for every instant — the fingerprint makes that visible.
        sql_fingerprint: plan ? crypto.createHash('md5').update(plan).digest('hex').slice(0, 12) : null,
        at: instant,
        graph: projectGraph(rows),
      });
    } catch (e) {
      json(res, 400, { ...pgError(e), elapsed_ms: Date.now() - started });
    } finally {
      await client.query('RESET og.valid_at; RESET og.system_at').catch(() => {});
      client.release();
    }
  },

  /**
   * What the time-travel bar offers: the graph's temporal types, and recent
   * writes from the audit log as "as we believed right after this" instants.
   */
  async 'GET /api/temporal'(req, res, url) {
    const graph = url.searchParams.get('graph') || 'default';
    try {
      const types = (await pool.query('SELECT og_temporal_types($1) AS t', [graph])).rows[0].t || [];
      const writes = (await pool.query(
        `SELECT at, actor, source, query FROM og_data.og_audit
          WHERE error_code IS NULL AND query ~* '\\m(CREATE|SET|DELETE|MERGE|REMOVE)\\M'
          ORDER BY at DESC LIMIT 30`
      )).rows;
      json(res, 200, { types, writes });
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /**
   * Create an entity type (optionally under parents). Schema changes rename,
   * re-parent and retype existing types; a new type is a plain catalog insert.
   */
  async 'POST /api/schema-type'(req, res) {
    const { graph = 'default', name = '', parents = [] } = await readBody(req);
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) return json(res, 400, { error: 'type name must be an identifier' });
    const started = Date.now();
    try {
      const r = await pool.query('SELECT og_create_type($1, $2, $3, $4::text[]) AS id',
        [graph, name, 'entity', parents]);
      json(res, 200, { type_id: r.rows[0].id, name, parents, elapsed_ms: Date.now() - started });
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /**
   * Turn a request in plain language into proposed schema changes.
   *
   * The model only proposes. Every change is checked here against the current
   * catalog — kind, type, declaring type of the property, identifier syntax —
   * and comes back marked ok or with the problem, for a person to review before
   * anything runs. Needs OPENAI_API_KEY; off otherwise.
   */
  async 'POST /api/schema-assist'(req, res) {
    const { graph = 'default', prompt = '' } = await readBody(req);
    if (!process.env.OPENAI_API_KEY) return json(res, 503, { error: 'AI assist is off — start the server with OPENAI_API_KEY' });
    if (!prompt.trim()) return json(res, 400, { error: 'empty request' });
    const started = Date.now();
    try {
      const snap = (await pool.query('SELECT og_schema_snapshot($1) AS s', [graph])).rows[0].s;
      const catalog = describeCatalog(snap);
      const system = ASSIST_SYSTEM.replace('{CATALOG}', catalog.text);
      const r = await fetch('https://api.openai.com/v1/responses', {
        method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${process.env.OPENAI_API_KEY}` },
        body: JSON.stringify({ model: process.env.OG_STUDIO_LLM_MODEL || 'gpt-4.1', instructions: system,
                               input: `${prompt}\n\n(Reply with the JSON object described above.)`, text: { format: { type: 'json_object' } } }),
      });
      const body = await r.json();
      if (!r.ok) return json(res, 502, { error: body.error?.message || 'model call failed' });
      const out = body.output_text || (body.output || []).flatMap((o) => o.content || []).map((c) => c.text || '').join('');
      let parsed; try { parsed = JSON.parse(out); } catch { return json(res, 502, { error: 'model returned non-JSON', raw: out.slice(0, 500) }); }
      const checked = checkChanges(parsed.changes || [], catalog);
      json(res, 200, { explanation: parsed.explanation || '', changes: checked,
                       model: process.env.OG_STUDIO_LLM_MODEL || 'gpt-4.1', elapsed_ms: Date.now() - started });
    } catch (e) {
      json(res, 400, { ...pgError(e), elapsed_ms: Date.now() - started });
    }
  },

  /** Recorded schema changes, newest first (spec 012). */
  async 'GET /api/schema-changes'(req, res, url) {
    const graph = url.searchParams.get('graph') || 'default';
    try {
      const r = await pool.query(
        `SELECT change_id, kind, phase, spec, report, created_at, updated_at
           FROM og_schema_changes($1) ORDER BY change_id DESC`, [graph]);
      json(res, 200, r.rows);
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /** The catalog as it stood at a version or an instant (current if neither). */
  async 'GET /api/schema-snapshot'(req, res, url) {
    const graph = url.searchParams.get('graph') || 'default';
    const version = url.searchParams.get('version');
    const at = url.searchParams.get('at');
    try {
      const r = await pool.query('SELECT og_schema_snapshot($1, $2::int8, $3::timestamptz) AS s',
        [graph, version || null, at || null]);
      json(res, 200, r.rows[0].s);
    } catch (e) {
      json(res, 400, pgError(e));
    }
  },

  /**
   * Drive one schema change through its phases: plan → expand → migrate* →
   * validate → contract (or abort), or `apply` for all of them in one
   * transaction. Every response carries how long the phase took and how many
   * rows of the affected types have been written since the change was planned —
   * the number that tells a catalog-level change from a rewrite.
   */
  async 'POST /api/schema-change'(req, res) {
    const { graph = 'default', action = '', change = null, change_id = null, batch = 10000 } = await readBody(req);
    const started = Date.now();
    try {
      // `apply` is a fresh change — never reuse an id left over from the panel
      let id = action === 'apply' ? null : change_id;
      let report;
      if (action === 'plan' || action === 'apply') {
        if (!change || !change.type) return json(res, 400, { error: 'change.type is required' });
        const xid = (await pool.query('SELECT pg_current_xact_id()::text::int8 AS x')).rows[0].x;
        if (action === 'plan') {
          id = (await pool.query('SELECT og_schema_plan($1, $2) AS c', [graph, change])).rows[0].c;
          report = (await pool.query('SELECT report FROM og_schema_changes($1) WHERE change_id = $2', [graph, id])).rows[0].report;
        } else {
          report = (await pool.query('SELECT og_schema_apply($1, $2) AS r', [graph, change])).rows[0].r;
        }
        changeWatch.set(id ?? `apply:${xid}`, { graph, type: change.type, xid });
        if (action === 'apply') id = `apply:${xid}`;
      } else {
        const fn = { expand: 'og_schema_expand', migrate: 'og_schema_migrate', validate: 'og_schema_validate',
                     contract: 'og_schema_contract', abort: 'og_schema_abort' }[action];
        if (!fn || !id) return json(res, 400, { error: 'action and change_id are required' });
        const sql = action === 'migrate' ? `SELECT ${fn}($1, $2) AS r` : `SELECT ${fn}($1) AS r`;
        report = (await pool.query(sql, action === 'migrate' ? [id, batch] : [id])).rows[0].r;
      }
      const elapsed_ms = Date.now() - started;
      const phase = typeof id === 'number'
        ? (await pool.query('SELECT phase FROM og_schema_changes($1) WHERE change_id = $2', [graph, id])).rows[0]?.phase
        : 'contracted';
      json(res, 200, { change_id: id, action, phase, report, elapsed_ms, storage: await rowsWritten(id) });
    } catch (e) {
      json(res, 400, { ...pgError(e), elapsed_ms: Date.now() - started });
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

  /**
   * Raw SQL escape hatch — this is still PostgreSQL underneath.
   *
   * Which is exactly why it is off by default: the pool holds credentials, so
   * this route is the pool's privileges handed to whoever can reach the port.
   */
  async 'POST /api/sql'(req, res) {
    if (!ALLOW_RAW_SQL) {
      return json(res, 403, {
        error: 'raw SQL is disabled — start the server with OG_STUDIO_ALLOW_RAW_SQL=1 to enable it',
      });
    }
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

  if (url.pathname.startsWith('/api/')) {
    if (!sameOrigin(req)) {
      return json(res, 403, { error: 'cross-origin request refused' });
    }
    if (req.method !== 'GET' && req.method !== 'HEAD' && !declaresJson(req)) {
      return json(res, 415, { error: 'content-type must be application/json' });
    }
  }

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

if (!LOOPBACK.has(HOST) && !ALLOW_REMOTE) {
  process.stderr.write(
    `refusing to bind ${HOST}: this server has no authentication, and the pool it holds ` +
      `can do anything its database role can.\nSet OG_STUDIO_ALLOW_REMOTE=1 if that is ` +
      `genuinely what you want, and put something in front of it that asks who is calling.\n`
  );
  process.exit(1);
}

server.listen(PORT, HOST, () => {
  // Print where it actually bound. The previous line said localhost no matter
  // what, which is how a public bind goes unnoticed.
  process.stdout.write(
    `Ontological Studio on http://${HOST}:${PORT}  ` +
      `(postgres ${pool.options.host}:${pool.options.port}/${pool.options.database})\n`
  );
  if (ALLOW_RAW_SQL) {
    process.stdout.write('  POST /api/sql is ENABLED — anyone who can reach this port can run any SQL\n');
  }
  if (!LOOPBACK.has(HOST)) {
    process.stdout.write(`  bound to ${HOST}, and nothing here authenticates\n`);
  }
});

process.on('SIGINT', () => {
  pool.end().finally(() => process.exit(0));
});
