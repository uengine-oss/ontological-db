/**
 * Benchmark report.
 *
 * Everything is derived from /api/benchmark, which reads the harness's own
 * result files. No latency is written into this file — if the page and the
 * measurements ever disagree, that is a bug here, not a stale copy.
 */
'use strict';

/* Column order puts this database first; `self` marks it for the emphasis
   encoding (one accent hue, everything else recessive). */
const SYSTEMS = [
  { key: 'ontological', label: 'Ontological', note: 'this database', self: true },
  { key: 'ontological_raw', label: 'Ontological', note: 'storage path', self: true, reuses: 'ontological' },
  { key: 'neo4j', label: 'Neo4j 5', note: 'native property graph' },
  { key: 'age', label: 'Apache AGE', note: 'Cypher on PostgreSQL' },
  { key: 'age_explicit', label: 'Apache AGE', note: 'explicit depths, no *1..n', reuses: 'age' },
  { key: 'typedb', label: 'TypeDB 3', note: 'typed entity–relation' },
  { key: 'cte', label: 'recursive CTE', note: 'plain SQL floor' },
];

const QUERIES = [
  ['1hop', '1 hop'],
  ['2hop', '2 hops'],
  ['3hop', '3 hops'],
  ['prop_scan', 'property scan'],
];

const HEADLINE = '3hop';

const $ = (s, r = document) => r.querySelector(s);
const el = (tag, cls, text) => {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
};

const ms = (v) =>
  v == null ? '—'
    : v >= 1000 ? `${Math.round(v).toLocaleString()} ms`
    : `${v.toFixed(2)} ms`;

const int = (v) => (v == null ? '—' : Math.round(v).toLocaleString());

const seconds = (s) =>
  s == null ? '—'
    : s >= 60 ? `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`
    : `${s.toFixed(1)} s`;

let DATA = null;
let active = 0;

init();

async function init() {
  try {
    DATA = await (await fetch('/api/benchmark')).json();
  } catch (e) {
    return fail(e.message);
  }
  if (!DATA.scales || !DATA.scales.length) {
    return fail('no benchmark results found — run bench/harness.py');
  }
  active = DATA.scales.length - 1; // lead with the largest graph
  buildSwitcher();
  render();
  fillMetrics();
}

function fail(msg) {
  $('#gate').className = 'gate bad';
  $('#gate').innerHTML = '';
  $('#gate').append(el('span', 'dot'), el('span', null, msg));
  $('#report').append(el('p', 'empty', 'No results to display.'));
}

function present(scale) {
  return SYSTEMS.filter((s) => scale.systems[s.key] && scale.systems[s.key].queries);
}

/* -------------------------------------------------------------- chrome --- */

function buildSwitcher() {
  const box = $('#scale-switch');
  DATA.scales.forEach((s, i) => {
    const b = el('button', null, `${s.scale.nodes.toLocaleString()} nodes`);
    b.setAttribute('aria-pressed', String(i === active));
    b.title = `${s.scale.edges.toLocaleString()} edges, average degree ${s.scale.avg_degree}`;
    b.addEventListener('click', () => {
      active = i;
      [...box.children].forEach((c, j) => c.setAttribute('aria-pressed', String(j === i)));
      render();
    });
    box.append(b);
  });
}

function renderGate(scale) {
  const gate = $('#gate');
  const checks = Object.entries(scale.correctness || {});
  const disagreed = checks.filter(([, c]) => !c.agree);
  const n = present(scale).length;
  gate.className = `gate ${disagreed.length ? 'bad' : 'ok'}`;
  gate.innerHTML = '';
  gate.append(el('span', 'dot'));
  if (disagreed.length) {
    gate.append(el('span', null,
      `${disagreed.map(([q]) => q).join(', ')} — systems disagreed, those timings are void`));
    return;
  }
  const answers = checks.map(([q, c]) => `${q} = ${c.value}`).join(', ');
  const b = el('b', null, `all ${n} systems returned identical answers`);
  gate.append(b, el('span', null, answers ? `(${answers})` : ''));
  if (scale.integrity_violations !== undefined) {
    gate.append(el('span', null, `· integrity violations: ${scale.integrity_violations}`));
  }
}

/* --------------------------------------------------------------- render --- */

function render() {
  const scale = DATA.scales[active];
  renderGate(scale);
  const report = $('#report');
  report.innerHTML = '';
  report.append(chartCard(scale), latencyCard(scale), costCard(scale));
}

/**
 * Emphasis bar chart of the headline query.
 *
 * The values span five orders of magnitude, so the linear domain is set by the
 * largest value still within 25× of the fastest; anything beyond that is drawn
 * running off the plot with its number spelled out, rather than squashing every
 * other bar into a pixel to accommodate it.
 */
function chartCard(scale) {
  const rows = present(scale)
    .map((s) => ({ ...s, m: scale.systems[s.key].queries[HEADLINE] }))
    .filter((r) => r.m && r.m.median_ms != null)
    .sort((a, b) => a.m.median_ms - b.m.median_ms);

  const card = el('section', 'card');
  card.append(el('h3', null, `${QUERIES.find(([k]) => k === HEADLINE)[1]} — median latency`));
  card.append(el('p', 'sub',
    `Distinct nodes within three hops of one node, ${scale.scale.nodes.toLocaleString()} nodes / ` +
    `${scale.scale.edges.toLocaleString()} edges. Lower is better.`));

  if (!rows.length) return card;

  const fastest = rows[0].m.median_ms;
  const inScale = rows.filter((r) => r.m.median_ms <= fastest * 25);
  const domain = inScale.length ? inScale[inScale.length - 1].m.median_ms : fastest;

  const chart = el('div', 'chart');
  for (const r of rows) {
    const off = r.m.median_ms > domain;
    const row = el('div', `row${r.self ? ' self' : ''}${off ? ' off' : ''}`);

    // Two rows say "Ontological" and two say "Apache AGE"; the qualifier is the
    // whole point of those pairs, so it is on the label, not in a tooltip.
    const name = el('div', 'name');
    name.append(el('span', 'nm', r.label));
    if (r.note) name.append(el('span', 'qual', r.note));

    const track = el('div', 'track');
    const bar = el('div', `bar${off ? ' clipped' : ''}`);
    bar.style.width = off ? '100%' : `${Math.max(1, (r.m.median_ms / domain) * 100)}%`;
    track.append(bar);
    if (off) track.append(el('span', 'breakmark', '⟩⟩'));

    row.append(name, track, el('div', 'val', ms(r.m.median_ms)));
    bindTip(row, r, scale);
    chart.append(row);
  }
  card.append(chart);

  const axis = el('div', 'axis');
  axis.append(el('span', null, '0'), el('span', null, ms(domain)));
  card.append(axis);

  const offCount = rows.filter((r) => r.m.median_ms > domain).length;
  if (offCount) {
    card.append(el('p', 'chart-note',
      `${offCount === 1 ? 'One bar runs' : `${offCount} bars run`} off the plot: ` +
      rows.filter((r) => r.m.median_ms > domain)
        .map((r) => `${r.label}${r.note ? ` (${r.note})` : ''} at ${ms(r.m.median_ms)}`)
        .join(', ') + '. The axis stops at ' + ms(domain) +
      ' so the rest stay readable.'));
  }
  return card;
}

function latencyCard(scale) {
  const cols = present(scale);
  const card = el('section', 'card');
  card.append(el('h3', null, 'Latency, all queries'));
  card.append(el('p', 'sub',
    'Median of the timed runs, five start nodes, warm cache. The floor row is what an ' +
    'empty query costs on the same client — Neo4j’s bolt round trip is several ' +
    'times psql’s, and at one hop that is most of its number.'));

  const rows = QUERIES.map(([key, label]) => ({
    label,
    values: cols.map((c) => scale.systems[c.key].queries[key]?.median_ms ?? null),
    best: true,
  }));
  rows.push({
    label: 'protocol floor',
    values: cols.map((c) => scale.systems[c.key].protocol_floor_ms ?? null),
    floor: true,
  });

  card.append(table(cols, rows));
  card.append(runNote(cols, scale));
  return card;
}

function costCard(scale) {
  const cols = present(scale);
  const card = el('section', 'card');
  card.append(el('h3', null, 'Load and page accesses'));
  card.append(el('p', 'sub',
    'Load is each system’s own practical bulk path — SQL for the PostgreSQL ' +
    'systems, batched UNWIND over bolt for Neo4j, one relation per query for ' +
    'TypeDB — so it says what it costs to get a graph in, not how fast each ' +
    'engine writes. Columns that read another system’s database load nothing ' +
    'and are left blank. Page counts exist only inside PostgreSQL.'));

  // A derived system reads a database another system loaded. Dividing the edge
  // count by its ~0-second no-op setup produces a number in the billions, which
  // is not a measurement of anything.
  const loaded = (c, field) =>
    c.reuses || scale.systems[c.key].reuses ? null : scale.systems[c.key][field] ?? null;

  const rows = [
    {
      label: 'load time',
      values: cols.map((c) => loaded(c, 'load_seconds')),
      fmt: seconds,
    },
    {
      label: 'edges / second',
      values: cols.map((c) => loaded(c, 'load_edges_per_sec')),
      fmt: int,
    },
    ...QUERIES.map(([key, label]) => ({
      label: `${label} — pages`,
      values: cols.map((c) => scale.systems[c.key].queries[key]?.buffers ?? null),
      fmt: int,
    })),
  ];
  card.append(table(cols, rows));
  return card;
}

function table(cols, rows) {
  const wrap = el('div', 'tablewrap');
  const t = el('table');

  const thead = el('thead');
  const hr = el('tr');
  hr.append(el('th', null, ''));
  for (const c of cols) {
    const th = el('th', c.self ? 'self' : null);
    th.append(document.createTextNode(c.label));
    if (c.note) {
      th.append(el('br'));
      const n = el('span', 'hint', c.note);
      n.style.cssText = 'font-weight:400;text-transform:none;letter-spacing:0;opacity:.7';
      th.append(n);
    }
    hr.append(th);
  }
  thead.append(hr);
  t.append(thead);

  const tb = el('tbody');
  for (const r of rows) {
    const tr = el('tr', r.floor ? 'floor' : null);
    tr.append(el('th', null, r.label));
    const fmt = r.fmt || ms;
    const numeric = r.values.filter((v) => v != null);
    const best = r.best && numeric.length ? Math.min(...numeric) : null;
    r.values.forEach((v, i) => {
      const td = el('td', [cols[i].self ? 'self' : '', v != null && v === best ? 'best' : '']
        .filter(Boolean).join(' ') || null, fmt(v));
      tr.append(td);
    });
    tb.append(tr);
  }
  t.append(tb);
  wrap.append(t);
  return wrap;
}

function runNote(cols, scale) {
  const p = el('p', 'runs');
  p.append(el('span', null, 'measured in:'));
  const seen = new Map();
  for (const c of cols) {
    const run = scale.runs?.[c.key];
    if (!run) continue;
    if (!seen.has(run)) seen.set(run, []);
    seen.get(run).push(c.key);
  }
  for (const [run, keys] of seen) {
    p.append(el('span', null, `${run} → ${keys.join(', ')}`));
  }
  return p;
}

/* -------------------------------------------------------------- tooltip --- */

function bindTip(node, row, scale) {
  const tip = $('#tip');
  const show = (e) => {
    tip.innerHTML = '';
    const title = el('b', null, `${row.label}${row.note ? ` — ${row.note}` : ''}`);
    const dl = el('dl');
    const add = (k, v) => { dl.append(el('dt', null, k), el('dd', null, v)); };
    add('median', ms(row.m.median_ms));
    add('p95', ms(row.m.p95_ms));
    add('min', ms(row.m.min_ms));
    add('runs', String(row.m.runs ?? '—'));
    if (row.m.buffers != null) add('pages', int(row.m.buffers));
    const engine = scale.systems[row.key]?.engine;
    tip.append(title, dl);
    if (engine) {
      const e = el('div', null, engine);
      e.style.cssText = 'margin-top:6px;color:#6b7787;font-size:11.5px';
      tip.append(e);
    }
    tip.classList.remove('hidden');
    move(e);
  };
  const move = (e) => {
    const pad = 14;
    const r = tip.getBoundingClientRect();
    let x = e.clientX + pad;
    let y = e.clientY + pad;
    if (x + r.width > innerWidth - 8) x = e.clientX - r.width - pad;
    if (y + r.height > innerHeight - 8) y = e.clientY - r.height - pad;
    tip.style.left = `${Math.max(8, x)}px`;
    tip.style.top = `${Math.max(8, y)}px`;
  };
  node.addEventListener('pointerenter', show);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerleave', () => tip.classList.add('hidden'));
}

/* ------------------------------------------ numbers quoted in the prose --- */

function fillMetrics() {
  const big = DATA.scales[DATA.scales.length - 1];
  const q = (sys, key) => big.systems[sys]?.queries?.[key]?.median_ms ?? null;
  const ratio = (a, b) => (a == null || b == null ? null : a / b);
  const times = (v) =>
    v == null ? '—' : v >= 100 ? `${Math.round(v).toLocaleString()}×` : `${v.toFixed(1)}×`;

  const metrics = {
    'neo4j-3hop': ms(q('neo4j', '3hop')),
    'og-3hop': ms(q('ontological', '3hop')),
    'og-vs-neo4j': times(ratio(q('ontological', '3hop'), q('neo4j', '3hop'))),
    'age-3hop': ms(q('age', '3hop')),
    'age-explicit-3hop': ms(q('age_explicit', '3hop')),
    'age-vle-factor': times(ratio(q('age', '3hop'), q('age_explicit', '3hop'))),
    'typedb-load': seconds(big.systems.typedb?.load_seconds),
    'neo4j-load': seconds(big.systems.neo4j?.load_seconds),
  };

  for (const [name, value] of Object.entries(metrics)) {
    document.querySelectorAll(`[data-metric="${name}"]`).forEach((n) => {
      n.textContent = value;
    });
  }
}
