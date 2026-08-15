/**
 * Ontological Studio — front end.
 *
 * A query stream of result frames, each with Graph / Table / SQL views, over a
 * canvas force layout. No build step and no dependencies: the whole thing is one
 * file you can read.
 */
'use strict';

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

const state = {
  graph: 'default',
  schema: null,
  stats: null,
  saved: JSON.parse(localStorage.getItem('og.saved') || '[]'),
  history: JSON.parse(localStorage.getItem('og.history') || '[]'),
  histIdx: -1,
};

/** Stable label colouring: the same type keeps its colour across frames. */
const PALETTE = [
  '#18b6a0', '#4c8bf5', '#e0a33e', '#c774e8', '#e0554c',
  '#5fbf6a', '#e07ab0', '#57c7e3', '#d9a441', '#8d8ff5',
];
const colorMemo = new Map();
function colorFor(label) {
  if (!colorMemo.has(label)) {
    let h = 0;
    for (const ch of String(label)) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    colorMemo.set(label, PALETTE[h % PALETTE.length]);
  }
  return colorMemo.get(label);
}

// -------------------------------------------------------------- transport ---

async function api(path, body, method = 'POST') {
  const res = await fetch(path, {
    method,
    headers: body ? { 'content-type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({ error: 'invalid response' }));
  return { ok: res.ok, data };
}

// ---------------------------------------------------------------- sidebar ---

async function loadHealth() {
  const { ok, data } = await api('/api/health', null, 'GET');
  $('#conn-dot').classList.toggle('ok', ok && data.ok);
  if (!ok || !data.ok) {
    $('#db-info').innerHTML =
      `<span style="color:var(--err)">not connected</span><br>${escapeHtml(data.error || '')}`;
    return;
  }
  const sel = $('#graph-select');
  sel.innerHTML = data.graphs.map((g) => `<option>${escapeHtml(g.name)}</option>`).join('');
  if (data.graphs.length) {
    state.graph = localStorage.getItem('og.graph') || data.graphs[0].name;
    if (!data.graphs.some((g) => g.name === state.graph)) state.graph = data.graphs[0].name;
    sel.value = state.graph;
  }
  $('#db-info').innerHTML = `
    <div>engine <b>ontological ${escapeHtml(data.version)}</b></div>
    <div>database <b>${escapeHtml(data.database)}</b></div>
    <div>${escapeHtml(data.server.split(',')[0])}</div>`;
  await loadSchema();
}

async function loadSchema() {
  const { ok, data } = await api(`/api/schema?graph=${encodeURIComponent(state.graph)}`, null, 'GET');
  if (!ok) return;
  state.schema = data.schema;
  state.stats = data.stats;

  const ents = data.schema.entity_types || [];
  const rels = data.schema.relation_types || [];

  $('#entity-chips').innerHTML = ents
    .map(
      (t) => `<span class="chip" style="color:${colorFor(t.name)}"
        data-run="MATCH (n:${t.name}) RETURN n LIMIT 50"
        title="${t.abstract ? 'abstract — matches subtypes only' : ''}">${escapeHtml(t.name)}${
        t.abstract ? ' ∗' : ''
      }<span class="n">${t.instances}</span></span>`
    )
    .join('') || '<span class="chip subtle">no entity types yet</span>';

  $('#rel-chips').innerHTML = rels
    .map(
      (t) => `<span class="chip" style="color:${colorFor(t.name)}"
        data-run="MATCH (a)-[r:${t.name}]->(b) RETURN a, r, b LIMIT 50">${escapeHtml(
        t.name
      )}<span class="n">${t.instances}</span></span>`
    )
    .join('') || '<span class="chip subtle">no relationship types yet</span>';

  const props = new Set();
  for (const t of [...ents, ...rels]) for (const p of t.properties || []) props.add(p.name);
  $('#prop-chips').innerHTML =
    [...props].sort().map((p) => `<span class="chip">${escapeHtml(p)}</span>`).join('') ||
    '<span class="chip">—</span>';

  renderHierarchy(ents, rels);
}

/** The type tree is the thing this database has that a label store does not. */
function renderHierarchy(ents, rels) {
  const all = [...ents, ...rels];
  const byName = new Map(all.map((t) => [t.name, t]));
  const children = new Map();
  const roots = [];
  for (const t of all) {
    const parents = (t.extends || []).filter((p) => byName.has(p));
    if (!parents.length) roots.push(t.name);
    for (const p of parents) {
      if (!children.has(p)) children.set(p, []);
      children.get(p).push(t.name);
    }
  }
  const out = [];
  const walk = (name, depth, seen) => {
    if (seen.has(name)) return;
    seen.add(name);
    const t = byName.get(name);
    const pad = '  '.repeat(depth) + (depth ? '└ ' : '');
    out.push(
      `<div class="t ${t.abstract ? 'abs' : ''}" data-run="MATCH (n:${name}) RETURN n LIMIT 50"
        style="color:${depth === 0 ? colorFor(name) : ''}">${escapeHtml(pad + name)}` +
        `<span style="opacity:.5"> ${t.instances}</span></div>`
    );
    for (const c of (children.get(name) || []).sort()) walk(c, depth + 1, seen);
  };
  for (const r of roots.sort()) walk(r, 0, new Set());
  $('#hierarchy').innerHTML = out.join('') || '<span style="opacity:.5">no types</span>';
}

async function loadAudit() {
  const { ok, data } = await api('/api/audit', null, 'GET');
  if (!ok) return;
  $('#audit-list').innerHTML = data
    .map((r) => {
      const q = String(r.query || '').replace(/^\[[^\]]*\]\s*/, '');
      return `<div class="row" data-run="${escapeHtml(q)}">
        <div class="q">${escapeHtml(q)}</div>
        <div class="meta ${r.error_code ? 'bad' : ''}">
          ${r.error_code ? 'error' : `${r.rows_out} rows`} ·
          ${Number(r.duration_ms).toFixed(1)} ms ·
          ${new Date(r.at).toLocaleTimeString()}
        </div></div>`;
    })
    .join('') || '<div class="pane">no queries yet</div>';
}

function renderSaved() {
  $('#saved-list').innerHTML =
    state.saved
      .map(
        (q, i) => `<div class="row" data-run="${escapeHtml(q)}">
          <div class="q">${escapeHtml(q)}</div>
          <div class="meta"><span class="unsave" data-i="${i}">remove</span></div></div>`
      )
      .join('') || '<div class="pane">star a frame to save its query</div>';
}

// ------------------------------------------------------------------ frames ---

let frameSeq = 0;

function addFrame(title) {
  const id = `f${++frameSeq}`;
  const el = document.createElement('div');
  el.className = 'frame';
  el.id = id;
  el.innerHTML = `
    <header>
      <span class="qtext">${escapeHtml(title)}</span>
      <div class="tabs"></div>
      <button class="save mini" title="save query">★</button>
      <button class="close" title="close">✕</button>
    </header>
    <div class="body"><div class="empty">running…</div></div>
    <div class="status"></div>`;
  $('#stream').prepend(el);
  el.querySelector('.close').onclick = () => el.remove();
  el.querySelector('.save').onclick = () => {
    if (!state.saved.includes(title)) {
      state.saved.unshift(title);
      localStorage.setItem('og.saved', JSON.stringify(state.saved.slice(0, 50)));
      renderSaved();
    }
  };
  return el;
}

function setViews(frame, views, status) {
  const tabs = frame.querySelector('.tabs');
  const body = frame.querySelector('.body');
  tabs.innerHTML = '';
  body.innerHTML = '';
  views.forEach((v, i) => {
    const tab = document.createElement('button');
    tab.className = 'tab' + (i === 0 ? ' active' : '');
    tab.textContent = v.name;
    tab.onclick = () => {
      $$('.tab', tabs).forEach((t) => t.classList.remove('active'));
      $$('.view', body).forEach((x) => x.classList.remove('active'));
      tab.classList.add('active');
      body.children[i].classList.add('active');
      if (v.onShow) v.onShow(body.children[i]);
    };
    tabs.appendChild(tab);

    const pane = document.createElement('div');
    pane.className = 'view' + (i === 0 ? ' active' : '');
    pane.appendChild(v.node);
    body.appendChild(pane);
  });
  frame.querySelector('.status').innerHTML = status;
  if (views[0]?.onShow) views[0].onShow(body.children[0]);
}

// ------------------------------------------------------------------- views ---

function tableView(rows, columns) {
  const wrap = document.createElement('div');
  wrap.className = 'tbl-wrap';
  if (!rows.length) {
    wrap.innerHTML = '<div class="empty">no rows</div>';
    return wrap;
  }
  const cols = columns.length ? columns : Object.keys(rows[0]);
  wrap.innerHTML = `<table class="rows">
    <thead><tr>${cols.map((c) => `<th>${escapeHtml(c)}</th>`).join('')}</tr></thead>
    <tbody>${rows
      .map(
        (r) =>
          `<tr>${cols.map((c) => `<td>${renderValue(r[c])}</td>`).join('')}</tr>`
      )
      .join('')}</tbody></table>`;
  return wrap;
}

function renderValue(v) {
  if (v === null || v === undefined) return '<span class="nul">null</span>';
  if (typeof v === 'number') return `<span class="num">${v}</span>`;
  if (typeof v === 'boolean') return `<span class="num">${v}</span>`;
  if (typeof v === 'string') return `<span class="str">${escapeHtml(v)}</span>`;
  if (v && v._id !== undefined) {
    const props = Object.entries(v)
      .filter(([k]) => !k.startsWith('_'))
      .map(([k, val]) => `${k}: ${JSON.stringify(val)}`)
      .join(', ');
    return `<span style="color:${colorFor(v._type)}">(${escapeHtml(v._type)})</span> {${escapeHtml(
      props
    )}}`;
  }
  return escapeHtml(JSON.stringify(v));
}

function codeView(text, kind) {
  const pre = document.createElement('pre');
  pre.className = 'code';
  pre.innerHTML = kind === 'sql' ? highlightSql(text) : escapeHtml(text);
  return pre;
}

function highlightSql(sql) {
  const kw =
    /\b(SELECT|FROM|WHERE|JOIN|LEFT|CROSS|LATERAL|ON|AND|OR|NOT|GROUP BY|ORDER BY|LIMIT|OFFSET|UNION ALL|UNION|WITH|RECURSIVE|AS|DISTINCT|IS|NULL|ANY|EXISTS|CASE|WHEN|THEN|ELSE|END)\b/g;
  return escapeHtml(sql)
    .replace(kw, '<span class="kw">$1</span>')
    .replace(/\b(jsonb_build_object|unnest|og_vlp|og_node_json|og_edge_json|to_jsonb|count|coalesce)\b/g, '<span class="fn">$1</span>')
    .replace(/('(?:[^']|'')*')/g, '<span class="lit">$1</span>');
}

function errorView(err, query) {
  const box = document.createElement('div');
  box.className = 'err-box';
  const msg = err.error || 'unknown error';
  box.innerHTML = `<div class="msg">${escapeHtml(msg)}</div>`;

  // The engine puts correction candidates in the message; surface them as buttons.
  const m = /did you mean:\s*(.+?)\.?$/m.exec(msg);
  if (m) {
    const wrong = /'([^']+)'/.exec(msg)?.[1];
    const fix = document.createElement('div');
    fix.className = 'fix';
    fix.innerHTML = '<b>Did you mean</b>';
    const row = document.createElement('div');
    row.className = 'sugg';
    for (const cand of m[1].split(',').map((s) => s.trim())) {
      const b = document.createElement('span');
      b.className = 'chip';
      b.style.color = colorFor(cand);
      b.textContent = cand;
      b.onclick = () => {
        $('#editor').value = wrong ? query.split(wrong).join(cand) : query;
        run();
      };
      row.appendChild(b);
    }
    fix.appendChild(row);
    box.appendChild(fix);
  }
  return box;
}

// ------------------------------------------------------- graph visualiser ---

function graphView(graph) {
  const wrap = document.createElement('div');
  wrap.className = 'graph-view';
  if (!graph.nodes.length) {
    wrap.innerHTML =
      '<div class="empty">no graph in this result — return nodes or relationships to draw one</div>';
    return { node: wrap };
  }

  const labels = [...new Set(graph.nodes.map((n) => n._type))];
  wrap.innerHTML = `
    <div class="legend">${labels
      .map(
        (l) =>
          `<span class="chip" style="color:${colorFor(l)};border-color:${colorFor(l)}">${escapeHtml(
            l
          )}</span>`
      )
      .join('')}</div>
    <canvas></canvas>
    <div class="gcontrols">
      <button data-act="fit" title="fit to view">⤢</button>
      <button data-act="freeze" title="pause layout">❙❙</button>
    </div>`;

  const canvas = wrap.querySelector('canvas');
  const sim = createSimulation(canvas, graph);
  wrap.querySelector('[data-act="fit"]').onclick = () => sim.fit();
  wrap.querySelector('[data-act="freeze"]').onclick = (e) => {
    sim.toggle();
    e.target.textContent = sim.running ? '❙❙' : '▶';
  };
  return { node: wrap, onShow: () => sim.resize() };
}

/**
 * Force-directed layout: repulsion between every pair, springs along edges,
 * gentle centring. Velocity-Verlet with damping, drawn on canvas so a few
 * hundred nodes stay smooth.
 */
function createSimulation(canvas, data) {
  const ctx = canvas.getContext('2d');
  const nodes = data.nodes.map((n, i) => ({
    d: n,
    id: String(n._id),
    x: Math.cos((i / data.nodes.length) * 6.283) * 120 + (Math.random() - 0.5) * 40,
    y: Math.sin((i / data.nodes.length) * 6.283) * 120 + (Math.random() - 0.5) * 40,
    vx: 0,
    vy: 0,
    r: 22,
  }));
  const index = new Map(nodes.map((n) => [n.id, n]));
  const edges = data.edges
    .map((e) => ({ d: e, s: index.get(String(e._src)), t: index.get(String(e._dst)) }))
    .filter((e) => e.s && e.t);

  let view = { x: 0, y: 0, k: 1 };
  let running = true;
  let alpha = 1;
  let dragging = null;
  let panning = null;
  let hover = null;
  let selected = null;

  function resize() {
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, rect.width * dpr);
    canvas.height = Math.max(1, rect.height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  function step() {
    if (!running || alpha < 0.005) return;
    const REPULSE = 5200;
    const SPRING = 0.012;
    const REST = 130;

    for (let i = 0; i < nodes.length; i++) {
      const a = nodes[i];
      for (let j = i + 1; j < nodes.length; j++) {
        const b = nodes[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 1) {
          dx = Math.random() - 0.5;
          dy = Math.random() - 0.5;
          d2 = 1;
        }
        const d = Math.sqrt(d2);
        const f = REPULSE / d2;
        const fx = (dx / d) * f;
        const fy = (dy / d) * f;
        a.vx -= fx;
        a.vy -= fy;
        b.vx += fx;
        b.vy += fy;
      }
    }
    for (const e of edges) {
      const dx = e.t.x - e.s.x;
      const dy = e.t.y - e.s.y;
      const d = Math.hypot(dx, dy) || 1;
      const f = (d - REST) * SPRING;
      const fx = (dx / d) * f;
      const fy = (dy / d) * f;
      e.s.vx += fx;
      e.s.vy += fy;
      e.t.vx -= fx;
      e.t.vy -= fy;
    }
    for (const n of nodes) {
      n.vx -= n.x * 0.004;
      n.vy -= n.y * 0.004;
      if (n === dragging) {
        n.vx = n.vy = 0;
        continue;
      }
      n.vx *= 0.82;
      n.vy *= 0.82;
      n.x += n.vx * alpha;
      n.y += n.vy * alpha;
    }
    alpha *= 0.994;
  }

  function caption(d) {
    for (const k of ['name', 'title', 'label', 'model', 'id']) {
      if (d[k] != null) return String(d[k]);
    }
    return d._type || String(d._id);
  }

  function draw() {
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    ctx.clearRect(0, 0, w, h);
    ctx.save();
    ctx.translate(w / 2 + view.x, h / 2 + view.y);
    ctx.scale(view.k, view.k);

    ctx.lineWidth = 1.2;
    for (const e of edges) {
      const c = colorFor(e.d._type);
      ctx.strokeStyle = c + '99';
      ctx.beginPath();
      ctx.moveTo(e.s.x, e.s.y);
      ctx.lineTo(e.t.x, e.t.y);
      ctx.stroke();

      // arrow head just outside the target circle
      const dx = e.t.x - e.s.x;
      const dy = e.t.y - e.s.y;
      const d = Math.hypot(dx, dy) || 1;
      const ax = e.t.x - (dx / d) * (e.t.r + 3);
      const ay = e.t.y - (dy / d) * (e.t.r + 3);
      const ang = Math.atan2(dy, dx);
      ctx.fillStyle = c;
      ctx.beginPath();
      ctx.moveTo(ax, ay);
      ctx.lineTo(ax - 8 * Math.cos(ang - 0.4), ay - 8 * Math.sin(ang - 0.4));
      ctx.lineTo(ax - 8 * Math.cos(ang + 0.4), ay - 8 * Math.sin(ang + 0.4));
      ctx.closePath();
      ctx.fill();

      if (view.k > 0.75) {
        ctx.fillStyle = '#93a1b3';
        ctx.font = '10px ui-monospace, monospace';
        ctx.textAlign = 'center';
        ctx.fillText(e.d._type, (e.s.x + e.t.x) / 2, (e.s.y + e.t.y) / 2 - 4);
      }
    }

    for (const n of nodes) {
      const c = colorFor(n.d._type);
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.r, 0, 6.2832);
      ctx.fillStyle = c;
      ctx.globalAlpha = n === hover || n === selected ? 1 : 0.88;
      ctx.fill();
      ctx.globalAlpha = 1;
      if (n === selected) {
        ctx.strokeStyle = '#fff';
        ctx.lineWidth = 2.5;
        ctx.stroke();
      }
      ctx.fillStyle = '#0d1014';
      ctx.font = '600 10px -apple-system, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      const cap = caption(n.d);
      ctx.fillText(cap.length > 9 ? cap.slice(0, 8) + '…' : cap, n.x, n.y);
    }
    ctx.restore();
  }

  function toWorld(ev) {
    const rect = canvas.getBoundingClientRect();
    return {
      x: (ev.clientX - rect.left - rect.width / 2 - view.x) / view.k,
      y: (ev.clientY - rect.top - rect.height / 2 - view.y) / view.k,
    };
  }
  function pick(p) {
    return nodes.find((n) => (n.x - p.x) ** 2 + (n.y - p.y) ** 2 < n.r * n.r);
  }

  canvas.addEventListener('mousedown', (ev) => {
    const p = toWorld(ev);
    const n = pick(p);
    if (n) {
      dragging = n;
      selected = n;
      showInspector(n.d);
      alpha = Math.max(alpha, 0.35);
    } else {
      panning = { x: ev.clientX - view.x, y: ev.clientY - view.y };
    }
  });
  window.addEventListener('mousemove', (ev) => {
    if (dragging) {
      const p = toWorld(ev);
      dragging.x = p.x;
      dragging.y = p.y;
      alpha = Math.max(alpha, 0.25);
    } else if (panning) {
      view.x = ev.clientX - panning.x;
      view.y = ev.clientY - panning.y;
    } else if (canvas.isConnected) {
      hover = pick(toWorld(ev));
      canvas.style.cursor = hover ? 'pointer' : 'grab';
    }
  });
  window.addEventListener('mouseup', () => {
    dragging = null;
    panning = null;
  });
  canvas.addEventListener('wheel', (ev) => {
    ev.preventDefault();
    view.k = Math.min(3, Math.max(0.2, view.k * (ev.deltaY < 0 ? 1.12 : 0.89)));
  }, { passive: false });
  canvas.addEventListener('dblclick', async (ev) => {
    const n = pick(toWorld(ev));
    if (!n) return;
    const { ok, data } = await api('/api/expand', { id: n.d._id, limit: 40 });
    if (!ok) return;
    for (const nd of data.nodes) {
      const key = String(nd._id);
      if (index.has(key)) continue;
      const fresh = {
        d: nd, id: key,
        x: n.x + (Math.random() - 0.5) * 90,
        y: n.y + (Math.random() - 0.5) * 90,
        vx: 0, vy: 0, r: 22,
      };
      nodes.push(fresh);
      index.set(key, fresh);
    }
    for (const ed of data.edges) {
      const s = index.get(String(ed._src));
      const t = index.get(String(ed._dst));
      if (s && t && !edges.some((e) => String(e.d._id) === String(ed._id))) {
        edges.push({ d: ed, s, t });
      }
    }
    alpha = 1;
    running = true;
  });

  function fit() {
    if (!nodes.length) return;
    const xs = nodes.map((n) => n.x);
    const ys = nodes.map((n) => n.y);
    const w = Math.max(...xs) - Math.min(...xs) + 120;
    const h = Math.max(...ys) - Math.min(...ys) + 120;
    view.k = Math.min(2, Math.max(0.2, Math.min(canvas.clientWidth / w, canvas.clientHeight / h)));
    view.x = -((Math.max(...xs) + Math.min(...xs)) / 2) * view.k;
    view.y = -((Math.max(...ys) + Math.min(...ys)) / 2) * view.k;
  }

  resize();
  const ro = new ResizeObserver(resize);
  ro.observe(canvas);
  // The canvas is created before setViews() attaches it, so "not connected" is
  // the normal state on the first few frames — only stop once it has been in
  // the document and then removed.
  let wasConnected = false;
  (function loop() {
    if (canvas.isConnected) {
      if (!wasConnected) {
        wasConnected = true;
        resize();
        alpha = 1;
      }
      step();
      draw();
    } else if (wasConnected) {
      ro.disconnect();
      return;
    }
    requestAnimationFrame(loop);
  })();

  return {
    resize,
    fit,
    get running() {
      return running;
    },
    toggle() {
      running = !running;
      if (running) alpha = Math.max(alpha, 0.3);
    },
  };
}

function showInspector(d) {
  $('#inspector').classList.remove('hidden');
  $('#insp-title').innerHTML = `<span style="color:${colorFor(d._type)}">${escapeHtml(
    d._type
  )}</span> · ${d._id}`;
  const rows = Object.entries(d)
    .filter(([k]) => k !== '_type')
    .map(
      ([k, v]) =>
        `<div class="k">${escapeHtml(k)}</div><div class="v">${escapeHtml(
          typeof v === 'string' ? v : JSON.stringify(v)
        )}</div>`
    )
    .join('');
  $('#insp-body').innerHTML =
    `<div class="kv">${rows}</div>
     <div class="actions">
       <button class="mini" id="insp-expand">expand neighbours</button>
       <button class="mini" id="insp-history">history</button>
     </div>`;
  $('#insp-expand').onclick = () => {
    $('#editor').value = `MATCH (n)-[r]-(m) WHERE id(n) = ${d._id} RETURN n, r, m LIMIT 50`;
    run();
  };
  $('#insp-history').onclick = () => {
    $('#editor').value = `:sql SELECT * FROM og_history(${d._id})`;
    run();
  };
}

// ------------------------------------------------------------------- runner ---

async function run() {
  const query = $('#editor').value.trim();
  if (!query) return;

  state.history.unshift(query);
  state.history = state.history.slice(0, 100);
  state.histIdx = -1;
  localStorage.setItem('og.history', JSON.stringify(state.history));

  if (query === ':clear') {
    $('#stream').innerHTML = '';
    $('#editor').value = '';
    return;
  }

  const frame = addFrame(query);
  $('#editor').value = '';
  autosize();

  if (query.startsWith(':sql ')) return runSql(frame, query.slice(5));
  if (query.startsWith(':explain ')) return runExplain(frame, query.slice(9));
  if (query.startsWith(':why ')) return runWhy(frame, query.slice(5));
  if (query === ':schema') return runSchemaCmd(frame);

  const t0 = performance.now();
  const { ok, data } = await api('/api/cypher', { graph: state.graph, query, params: {} });
  const ms = (performance.now() - t0).toFixed(0);

  if (!ok) {
    setViews(
      frame,
      [{ name: 'Error', node: errorView(data, query) }],
      `<span class="bad">failed</span><span>${ms} ms</span>`
    );
    loadAudit();
    return;
  }

  const views = [];
  if (data.graph.nodes.length) {
    const gv = graphView(data.graph);
    views.push({ name: 'Graph', node: gv.node, onShow: gv.onShow });
  }
  views.push({ name: 'Table', node: tableView(data.rows, data.columns) });
  views.push({ name: 'JSON', node: codeView(JSON.stringify(data.rows, null, 2)) });
  if (data.compiled_sql) {
    views.push({ name: 'SQL', node: codeView(data.compiled_sql, 'sql') });
  }

  setViews(
    frame,
    views,
    `<span class="ok">${data.rows.length} row${data.rows.length === 1 ? '' : 's'}</span>
     <span>${data.graph.nodes.length} nodes · ${data.graph.edges.length} relationships</span>
     <span>server ${data.elapsed_ms} ms · round trip ${ms} ms</span>`
  );
  loadSchema();
  loadAudit();
}

async function runSql(frame, sql) {
  const { ok, data } = await api('/api/sql', { sql });
  if (!ok) {
    setViews(frame, [{ name: 'Error', node: errorView(data, sql) }], '<span class="bad">failed</span>');
    return;
  }
  setViews(
    frame,
    [
      { name: 'Table', node: tableView(data.rows, data.columns) },
      { name: 'JSON', node: codeView(JSON.stringify(data.rows, null, 2)) },
    ],
    `<span class="ok">${data.rowCount} rows</span><span>raw SQL</span>`
  );
}

async function runExplain(frame, query) {
  const { ok, data } = await api('/api/explain', { graph: state.graph, query, analyze: false });
  if (!ok) {
    setViews(frame, [{ name: 'Error', node: errorView(data, query) }], '<span class="bad">failed</span>');
    return;
  }
  setViews(
    frame,
    [
      { name: 'SQL', node: codeView(data.sql, 'sql') },
      { name: 'Plan', node: codeView(JSON.stringify(data.plan, null, 2)) },
    ],
    `<span>columns: ${(data.columns || []).join(', ')}</span>`
  );
}

async function runWhy(frame, query) {
  const { ok, data } = await api('/api/diagnose', { graph: state.graph, query });
  if (!ok) {
    setViews(frame, [{ name: 'Error', node: errorView(data, query) }], '<span class="bad">failed</span>');
    return;
  }
  const box = document.createElement('div');
  box.className = 'err-box';
  const steps = data.empty?.steps || [];
  box.innerHTML =
    `<div class="fix"><b>Compile check</b><br>${
      data.error?.ok ? 'query compiles' : escapeHtml(data.error?.message || '')
    }</div>` +
    (steps.length
      ? `<div class="fix"><b>Pattern walk</b>${steps
          .map((s) =>
            s.verdict
              ? `<div style="color:var(--warn)">${escapeHtml(s.verdict)} — ${escapeHtml(s.hint || '')}</div>`
              : `<div><code>${escapeHtml(s.description)}</code> → ${s.rows} rows</div>`
          )
          .join('')}</div>`
      : '') +
    (data.estimate
      ? `<div class="fix"><b>Estimate</b><br>${Math.round(
          data.estimate.estimated_rows || 0
        )} rows, cost ${Math.round(data.estimate.estimated_cost || 0)}${
          (data.estimate.advice || []).length
            ? '<br>' + data.estimate.advice.map(escapeHtml).join('<br>')
            : ''
        }</div>`
      : '');
  setViews(frame, [{ name: 'Diagnosis', node: box }], '<span>diagnostics</span>');
}

async function runSchemaCmd(frame) {
  const { ok, data } = await api(`/api/schema?graph=${encodeURIComponent(state.graph)}`, null, 'GET');
  if (!ok) return;
  setViews(
    frame,
    [
      { name: 'Schema', node: codeView(JSON.stringify(data.schema, null, 2)) },
      { name: 'Storage', node: codeView(JSON.stringify(data.stats, null, 2)) },
    ],
    '<span>machine-readable schema — this is what an agent sees</span>'
  );
}

// -------------------------------------------------------------------- misc ---

function escapeHtml(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]
  );
}

function autosize() {
  const ta = $('#editor');
  ta.style.height = 'auto';
  ta.style.height = Math.min(ta.scrollHeight, window.innerHeight * 0.4) + 'px';
}

// ------------------------------------------------------------------- wiring ---

$('#run-btn').onclick = run;
$('#editor').addEventListener('input', autosize);
$('#editor').addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    run();
  } else if (e.key === 'ArrowUp' && !$('#editor').value.includes('\n')) {
    if (state.histIdx + 1 < state.history.length) {
      state.histIdx++;
      $('#editor').value = state.history[state.histIdx];
      autosize();
      e.preventDefault();
    }
  } else if (e.key === 'ArrowDown' && state.histIdx >= 0) {
    state.histIdx--;
    $('#editor').value = state.histIdx >= 0 ? state.history[state.histIdx] : '';
    autosize();
    e.preventDefault();
  }
});

// `[data-panel]` skips the rail's plain links — they navigate, they don't
// switch panels.
$$('.rail-btn[data-panel]').forEach((btn) => {
  btn.onclick = () => {
    $$('.rail-btn').forEach((b) => b.classList.remove('active'));
    btn.classList.add('active');
    $$('.panel').forEach((p) =>
      p.classList.toggle('hidden', p.dataset.panel !== btn.dataset.panel)
    );
    if (btn.dataset.panel === 'audit') loadAudit();
    if (btn.dataset.panel === 'saved') renderSaved();
  };
});

$('#graph-select').onchange = (e) => {
  state.graph = e.target.value;
  localStorage.setItem('og.graph', state.graph);
  loadSchema();
};

$('#insp-close').onclick = () => $('#inspector').classList.add('hidden');

// Anything with data-run becomes a one-click query.
document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-run]');
  if (el) {
    $('#editor').value = el.dataset.run;
    autosize();
    run();
    return;
  }
  const un = e.target.closest('.unsave');
  if (un) {
    e.stopPropagation();
    state.saved.splice(Number(un.dataset.i), 1);
    localStorage.setItem('og.saved', JSON.stringify(state.saved));
    renderSaved();
  }
});

loadHealth();
renderSaved();
