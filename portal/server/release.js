'use strict';
/**
 * Releases — moving a schema and its data from one Ontological database to
 * another, as a recorded, versioned step.
 *
 * The database does the deciding (engine/sql/release.sql): `og_schema_diff`
 * says which schema changes would make this graph match a package,
 * `og_release_begin/commit` keep the record, `og_timeline` reads an element's
 * validity slices. This module is the management API over those — the same
 * place the schema-change and time-travel endpoints live — so a client never
 * needs SQL access to reach them.
 *
 * A package is plain JSON:
 *
 *   { format: 'og-release/1', name, origin, graph, created_at,
 *     snapshot,            // og_schema_snapshot of the source graph
 *     changes,             // og_schema_changes of the source graph (rename hints)
 *     instances: [{ type, key_prop, key, slices: [{ valid_from, valid_to, props, source }] }],
 *     edges:     [{ type, from: { type, key_prop, key }, to: { … } }],
 *     hash }               // sha256 over everything above except name/created_at
 *
 * Instances travel as their validity slices, not just their current value, so
 * an amendment with a future effective date — or a correction of one — arrives
 * in the target with the same validity it had in the source.
 */

const crypto = require('node:crypto');

const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/;

function canonical(v) {
  if (Array.isArray(v)) return `[${v.map(canonical).join(',')}]`;
  if (v && typeof v === 'object') {
    return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canonical(v[k])}`).join(',')}}`;
  }
  return JSON.stringify(v ?? null);
}

function packageHash(pkg) {
  const { snapshot, instances, edges } = pkg;
  // Version numbers and timestamps differ between databases; the structure does not.
  const types = (snapshot?.types || []).map((t) => ({ ...t }));
  return 'sha256:' + crypto.createHash('sha256').update(canonical({ types, instances, edges })).digest('hex');
}

const iso = (d) => (d == null ? null : new Date(d).toISOString());
const finite = (d) => (d == null || !isFinite(new Date(d)) ? null : iso(d));
const same = (a, b) => canonical(a ?? null) === canonical(b ?? null);

module.exports = function register(routes, { pool, json, readBody, pgError }) {
  /** Key property of each type: from the request, else a property flagged `key`. */
  function keyMap(snapshot, keys = {}) {
    const out = {};
    for (const t of snapshot.types || []) {
      if (t.kind !== 'e' || t.abstract) continue;
      const declared = Object.entries(t.properties || {}).find(([, p]) => p.key)?.[0];
      const k = keys[t.name] || declared;
      if (k) out[t.name] = k;
    }
    // a subtype without its own key uses its parent's
    for (const t of snapshot.types || []) {
      if (t.kind === 'e' && !out[t.name]) {
        const viaParent = (t.parents || []).map((p) => keys[p] || out[p]).find(Boolean);
        if (viaParent) out[t.name] = viaParent;
      }
    }
    return out;
  }

  async function exportPackage(graph, { keys = {}, name = '', origin = '' } = {}) {
    const snapshot = (await pool.query('SELECT og_schema_snapshot($1) AS s', [graph])).rows[0].s || { types: [] };
    const changes = (await pool.query(
      'SELECT change_id, kind, phase, spec, created_by, created_at FROM og_schema_changes($1) ORDER BY change_id', [graph])).rows;
    const km = keyMap(snapshot, keys);
    const skipped = [];
    const instances = [];
    const byId = new Map();

    for (const t of snapshot.types || []) {
      if (t.kind !== 'e' || t.abstract) continue;
      const kp = km[t.name];
      if (!kp) { skipped.push({ type: t.name, reason: 'no key property' }); continue; }
      // elements of exactly this type — a subtype exports its own
      const tbl = (await pool.query(
        `SELECT t.storage_table, p.column_name FROM og_catalog.type t
           JOIN og_catalog.graph g USING (graph_id)
           LEFT JOIN og_catalog.property p ON p.type_id = t.type_id AND p.name = $3
          WHERE g.name = $1 AND t.name = $2`, [graph, t.name, kp])).rows[0];
      if (!tbl?.storage_table) continue;
      const keyExpr = tbl.column_name ? `x.${tbl.column_name}` : `x.__ext->>'${kp.replace(/'/g, "''")}'`;
      const ks = (await pool.query(`SELECT DISTINCT ${keyExpr}::text AS k FROM ${tbl.storage_table} x WHERE ${keyExpr} IS NOT NULL ORDER BY 1`)).rows;
      for (const { k } of ks) {
        const sl = (await pool.query(
          `SELECT id, valid_from, valid_to, props, source FROM og_timeline($1, $2, $3, $4)
            WHERE type_name = $2 ORDER BY valid_from`, [graph, t.name, kp, k])).rows;
        if (!sl.length) continue;
        const inst = {
          type: t.name, key_prop: kp, key: k,
          slices: sl.map((s) => ({ valid_from: finite(s.valid_from), valid_to: finite(s.valid_to), props: s.props, source: s.source })),
        };
        instances.push(inst);
        byId.set(String(sl[0].id), { type: t.name, key_prop: kp, key: k });
      }
    }

    const edges = [];
    for (const t of snapshot.types || []) {
      if (t.kind !== 'r') continue;
      const rows = (await pool.query('SELECT og_cypher($1, $2) AS r', [graph,
        `MATCH (a)-[r:${t.name}]->(b) RETURN id(a) AS a, id(b) AS b`])).rows.map((x) => x.r);
      for (const r of rows) {
        const from = byId.get(String(r.a)); const to = byId.get(String(r.b));
        if (from && to) edges.push({ type: t.name, from, to });
        else skipped.push({ type: t.name, reason: 'endpoint not exported', a: r.a, b: r.b });
      }
    }
    edges.sort((x, y) => canonical(x).localeCompare(canonical(y)));

    const pkg = { format: 'og-release/1', name, origin, graph, created_at: new Date().toISOString(),
                  schema_version: snapshot.version ?? null, snapshot, changes, instances, edges };
    pkg.hash = packageHash(pkg);
    return { pkg, skipped };
  }

  /** The target's value of one instance at an instant, or null when it has none then. */
  async function valueAt(client, graph, inst, at) {
    // No instant (a type without valid time): its single, current value.
    const r = at
      ? await client.query(
          `SELECT props FROM og_timeline($1, $2, $3, $4, $5::timestamptz, $5::timestamptz + interval '1 millisecond')
            WHERE type_name = $2 LIMIT 1`, [graph, inst.type, inst.key_prop, inst.key, at])
      : await client.query(
          `SELECT props FROM og_timeline($1, $2, $3, $4) WHERE type_name = $2 AND op = 'current' LIMIT 1`,
          [graph, inst.type, inst.key_prop, inst.key]);
    return r.rows[0]?.props ?? null;
  }

  function differs(slice, current) {
    if (!current) return Object.keys(slice.props || {});
    return Object.keys(slice.props || {}).filter((k) => !same(slice.props[k], current[k]));
  }

  async function planPackage(graph, pkg) {
    const diff = (await pool.query('SELECT og_schema_diff($1, $2, $3) AS d',
      [graph, pkg.snapshot, JSON.stringify(pkg.changes || [])])).rows[0].d;
    // Instances are compared against what the target holds once the schema
    // matches. A type that does not exist yet means every instance is new.
    const known = new Set(((await pool.query('SELECT og_schema_snapshot($1) AS s', [graph])).rows[0].s?.types || []).map((t) => t.name));
    // Compare against the target as it will be once the schema changes run:
    // a property renamed in the package is read under its new name here too.
    const renames = new Map(diff.filter((c) => c.kind === 'rename_property').map((c) => [c.property, c.new_name]));
    const renamed = (props) => props && Object.fromEntries(Object.entries(props).map(([k, v]) => [renames.get(k) || k, v]));
    const client = await pool.connect();
    const inst = { create: 0, update: 0, same: 0, slices: 0, items: [] };
    try {
      for (const i of pkg.instances || []) {
        let action = 'same'; const changed = [];
        for (const [n, s] of i.slices.entries()) {
          const cur = known.has(i.type) ? renamed(await valueAt(client, graph, i, s.valid_from)) : null;
          if (n === 0 && !cur) { action = 'create'; inst.slices++; continue; }
          const d = differs(s, cur);
          if (d.length) { if (action === 'same') action = 'update'; changed.push({ valid_from: s.valid_from, props: d }); inst.slices++; }
        }
        inst[action]++;
        if (action !== 'same') inst.items.push({ type: i.type, key: i.key, action, changed });
      }
    } finally { client.release(); }
    return { schema: diff, instances: inst, edges: (pkg.edges || []).length };
  }

  async function runChange(c, graph, log) {
    const step = async (phase, sql, args) => {
      const t0 = Date.now();
      const r = (await pool.query(sql, args)).rows[0].r;
      log.push({ kind: c.kind, type: c.type, property: c.property, phase, ms: Date.now() - t0, result: r });
      return r;
    };
    if (c.kind === 'create_type') {
      const kind = c.type_kind === 'r' || c.type_kind === 'relation' ? 'relation' : 'entity';
      await step('create', 'SELECT og_create_type($1, $2, $3, $4::text[]) AS r', [graph, c.type, kind, c.parents || []]);
      return null;
    }
    if (c.kind === 'enable_temporal') {
      await step('enable', "SELECT og_enable_temporal($1, $2) IS NULL AS r", [graph, c.type]);
      return null;
    }
    if (c.kind === 'drop_property') {
      log.push({ kind: c.kind, type: c.type, property: c.property, phase: 'skipped', result: 'destructive — not applied by a release' });
      return null;
    }
    const spec = { ...c }; delete spec.destructive;
    const id = await step('plan', 'SELECT og_schema_plan($1, $2) AS r', [graph, spec]);
    await step('expand', 'SELECT og_schema_expand($1) AS r', [id]);
    for (let i = 0; i < 10_000; i++) {
      const m = await step('migrate', 'SELECT og_schema_migrate($1, 10000) AS r', [id]);
      if (m.done) break;
    }
    const v = await step('validate', 'SELECT og_schema_validate($1) AS r', [id]);
    if (v.phase !== 'validated') {
      await step('abort', 'SELECT og_schema_abort($1) AS r', [id]);
      throw Object.assign(new Error(`change ${id} (${c.kind} ${c.type}.${c.property || ''}) has ${v.violations} violation(s)`), { samples: v.samples });
    }
    await step('contract', 'SELECT og_schema_contract($1) AS r', [id]);
    return id;
  }

  /** One write, in its own transaction, at a valid instant and under the release's source. */
  async function write(graph, query, params, ctx) {
    const client = await pool.connect();
    try {
      await client.query('BEGIN');
      if (ctx.valid_from) await client.query("SELECT set_config('og.valid_from', $1, true)", [ctx.valid_from]);
      if (ctx.valid_to) await client.query("SELECT set_config('og.valid_to', $1, true)", [ctx.valid_to]);
      await client.query("SELECT set_config('og.source', $1, true)", [ctx.source]);
      if (ctx.actor) await client.query("SELECT set_config('og.actor', $1, true)", [ctx.actor]);
      const r = await client.query('SELECT og_cypher($1, $2, $3) AS r', [graph, query, JSON.stringify(params)]);
      await client.query('COMMIT');
      return r.rows.map((x) => x.r);
    } catch (e) {
      await client.query('ROLLBACK').catch(() => {});
      throw e;
    } finally {
      client.release();
    }
  }

  function propsClause(props, prefix = 'p') {
    const keys = Object.keys(props).filter((k) => IDENT.test(k) && props[k] !== null);
    return { text: keys.map((k, i) => `${k}: $${prefix}${i}`).join(', '),
             params: Object.fromEntries(keys.map((k, i) => [`${prefix}${i}`, props[k]])) };
  }

  async function applyPackage(graph, pkg, actor) {
    if (pkg.hash !== packageHash(pkg)) throw new Error('package hash does not match its content');
    const plan = await planPackage(graph, pkg);
    // og.actor is read by og_release_begin; set it in the same statement's transaction
    const rid = (await pool.query(
      `SELECT og_release_begin($1, $2, $3, $4) AS r FROM (SELECT set_config('og.actor', COALESCE($5, ''), true)) s`,
      [graph, pkg.name || pkg.hash.slice(7, 19), pkg.origin || null, pkg.hash, actor])).rows[0].r;
    const source = `release:${rid}`;
    const log = []; const changeIds = []; let written = 0;
    try {
      for (const c of plan.schema) {
        const id = await runChange(c, graph, log);
        if (id) changeIds.push(id);
      }
      const client = await pool.connect();
      try {
        for (const i of pkg.instances || []) {
          const match = `MATCH (n:${i.type} {${i.key_prop}: $key})`;
          for (const [n, s] of i.slices.entries()) {
            const cur = await valueAt(client, graph, i, s.valid_from);
            // An end of validity travels only on the last slice: an earlier
            // slice ends where the next begins, and writing that next slice is
            // what closes it.
            const last = n === i.slices.length - 1;
            const ctx = { valid_from: s.valid_from, valid_to: last ? s.valid_to : null, source, actor };
            if (!cur) {
              const exists = (await client.query('SELECT og_cypher($1, $2, $3) AS r', [graph,
                `${match} RETURN count(n) AS c`, JSON.stringify({ key: i.key })])).rows[0]?.r?.c > 0;
              if (!exists) {
                const pc = propsClause(s.props);
                await write(graph, `CREATE (n:${i.type} {${pc.text}})`, pc.params, ctx);
                written++;
                continue;
              }
            }
            const d = differs(s, cur).filter((k) => IDENT.test(k));
            if (!d.length) continue;
            const sets = d.map((k, j) => `n.${k} = $v${j}`).join(', ');
            await write(graph, `${match} SET ${sets}`,
              { key: i.key, ...Object.fromEntries(d.map((k, j) => [`v${j}`, s.props[k] ?? null])) }, ctx);
            written++;
          }
        }
      } finally { client.release(); }
      let edges = 0;
      for (const e of pkg.edges || []) {
        const ends = `(a:${e.from.type} {${e.from.key_prop}: $a}), (b:${e.to.type} {${e.to.key_prop}: $b})`;
        const c = (await pool.query('SELECT og_cypher($1, $2, $3) AS r', [graph,
          `MATCH (a:${e.from.type} {${e.from.key_prop}: $a})-[r:${e.type}]->(b:${e.to.type} {${e.to.key_prop}: $b}) RETURN count(r) AS c`,
          JSON.stringify({ a: e.from.key, b: e.to.key })])).rows[0]?.r?.c || 0;
        if (c > 0) continue;
        await write(graph, `MATCH ${ends} CREATE (a)-[:${e.type}]->(b)`, { a: e.from.key, b: e.to.key }, { source, actor });
        edges++;
      }
      const summary = { schema: plan.schema.length, schema_log: log, instances: plan.instances, writes: written, edges_created: edges,
                        source_graph: pkg.graph, source_version: pkg.schema_version };
      const rec = (await pool.query('SELECT og_release_commit($1, $2, $3, $4) AS r',
        [rid, changeIds, written, summary])).rows[0].r;
      return { release: rec, plan, log };
    } catch (e) {
      await pool.query('SELECT og_release_commit($1, $2, $3, $4, true) AS r',
        [rid, changeIds, written, { error: e.message, schema_log: log }]).catch(() => {});
      e.release_id = rid; e.log = log;
      throw e;
    }
  }

  Object.assign(routes, {
    /** Turn on valid/system time for a type and its subtypes (spec 013). */
    async 'POST /api/schema-temporal'(req, res) {
      const { graph = 'default', type = '' } = await readBody(req);
      try {
        await pool.query('SELECT og_enable_temporal($1, $2)', [graph, type]);
        json(res, 200, { type, temporal: true });
      } catch (e) { json(res, 400, pgError(e)); }
    },

    /** What would make this graph's schema equal to a snapshot (no writes). */
    async 'POST /api/schema-diff'(req, res) {
      const { graph = 'default', snapshot = {}, hints = [] } = await readBody(req);
      try {
        const r = await pool.query('SELECT og_schema_diff($1, $2, $3) AS d', [graph, snapshot, JSON.stringify(hints)]);
        json(res, 200, { changes: r.rows[0].d });
      } catch (e) { json(res, 400, pgError(e)); }
    },

    /** Package this graph — schema, change log, instances with their validity, edges. */
    async 'POST /api/release/export'(req, res) {
      const { graph = 'default', keys = {}, name = '', origin = '' } = await readBody(req);
      try {
        const { pkg, skipped } = await exportPackage(graph, { keys, name, origin });
        json(res, 200, { package: pkg, skipped,
                         counts: { types: pkg.snapshot.types?.length || 0, instances: pkg.instances.length,
                                   slices: pkg.instances.reduce((n, i) => n + i.slices.length, 0), edges: pkg.edges.length } });
      } catch (e) { json(res, 400, pgError(e)); }
    },

    /** What applying a package here would do — schema changes and instance writes. Writes nothing. */
    async 'POST /api/release/plan'(req, res) {
      const { graph = 'default', package: pkg } = await readBody(req);
      if (!pkg?.snapshot) return json(res, 400, { error: 'package is required' });
      try {
        const applied = (await pool.query("SELECT r FROM og_releases($1) r WHERE r->>'package_hash' = $2 AND r->>'status' = 'applied'",
          [graph, pkg.hash])).rows[0]?.r || null;
        json(res, 200, { ...(await planPackage(graph, pkg)), hash_ok: pkg.hash === packageHash(pkg), already_applied: applied });
      } catch (e) { json(res, 400, pgError(e)); }
    },

    /** Apply a package as one release: schema changes phase by phase, then instances. */
    async 'POST /api/release/apply'(req, res) {
      const { graph = 'default', package: pkg, actor = null } = await readBody(req);
      if (!pkg?.snapshot) return json(res, 400, { error: 'package is required' });
      const t0 = Date.now();
      try {
        json(res, 200, { ...(await applyPackage(graph, pkg, actor)), elapsed_ms: Date.now() - t0 });
      } catch (e) {
        json(res, 400, { ...pgError(e), release_id: e.release_id ?? null, log: e.log ?? [], samples: e.samples ?? null });
      }
    },

    /** Releases applied to this graph, newest first. */
    async 'GET /api/releases'(req, res, url) {
      const graph = url.searchParams.get('graph') || 'default';
      try {
        json(res, 200, (await pool.query('SELECT r FROM og_releases($1) r', [graph])).rows.map((x) => x.r));
      } catch (e) { json(res, 400, pgError(e)); }
    },

    /** One element's validity slices over [from, to), as believed at system_at (now by default). */
    async 'GET /api/timeline'(req, res, url) {
      const q = Object.fromEntries(url.searchParams);
      try {
        const r = await pool.query(
          `SELECT type_name, id, valid_from, valid_to, props, recorded_at, superseded_at, source, actor, op
             FROM og_timeline($1, $2, $3, $4, COALESCE($5::timestamptz, '-infinity'), COALESCE($6::timestamptz, 'infinity'), $7::timestamptz)`,
          [q.graph || 'default', q.type || '', q.key_prop || '', q.key || '', q.from || null, q.to || null, q.system_at || null]);
        json(res, 200, { slices: r.rows.map((s) => ({ ...s, valid_from: finite(s.valid_from), valid_to: finite(s.valid_to) })) });
      } catch (e) { json(res, 400, pgError(e)); }
    },
  });
};

module.exports.packageHash = packageHash;
