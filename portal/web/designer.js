/**
 * Ontological Studio — Schema designer (spec 012, visual).
 *
 * The schema as a map: entity types as cards in their hierarchy, each with the
 * properties it declares (and the column each one maps to), relationship types
 * along the bottom. Changes are staged, never run on click — from the map
 * (click a property or a type, drag a card onto a new parent), from a plain-
 * language request the server turns into checked proposals, or as raw JSON.
 * Staged changes are drawn on the map before they run. Running a change walks
 * plan → expand → migrate → validate → contract and reports, per change, how
 * long it took and how many stored rows it rewrote.
 *
 * Loaded after app.js and uses its helpers ($, $$, api, state, escapeHtml,
 * loadSchema, loadEvo).
 */
'use strict';

const D = {
  open: false,
  snap: null,          // og_schema_snapshot as last read
  stats: new Map(),    // type → instance count
  plan: [],            // staged changes
  seq: 0,
  running: false,
};

const TYPE_CHOICES = ['int', 'float', 'string', 'bool', 'date', 'timestamp'];

// ------------------------------------------------------------------ model ---

function ancestorsOf(types, name, seen = new Set()) {
  for (const p of types.get(name)?.parents || []) if (!seen.has(p)) { seen.add(p); ancestorsOf(types, p, seen); }
  return seen;
}

/** Apply staged changes to a copy of the snapshot, marking what they do. */
function virtualSchema() {
  const types = new Map();
  for (const t of D.snap?.types || []) {
    types.set(t.name, {
      name: t.name, kind: t.kind, temporal: t.temporal, abstract: t.abstract,
      parents: [...(t.parents || [])], removedParents: [], addedParents: [],
      props: Object.entries(t.properties || {}).map(([k, v]) => ({ name: k, type: v.type, column: v.column, required: v.required })),
      count: D.stats.get(t.name) ?? 0, isNew: false, renamedFrom: null,
    });
  }
  // declared vs inherited, from the real hierarchy
  for (const t of types.values()) {
    const inherited = new Set([...ancestorsOf(types, t.name)].flatMap((a) => (types.get(a)?.props || []).map((p) => p.name)));
    t.props.forEach((p) => { p.inherited = inherited.has(p.name); });
  }
  for (const item of D.plan) {
    if (item.status === 'done') continue;
    const c = item.change;
    const t = types.get(c.type);
    const prop = t?.props.find((p) => p.name === c.property);
    const mark = (o, m) => { o.marks = [...(o.marks || []), { ...m, item: item.id }]; };
    switch (c.kind) {
      case 'create_type':
        types.set(c.type, { name: c.type, kind: 'e', parents: [...(c.parents || [])], removedParents: [], addedParents: [],
          props: [], count: 0, isNew: true, item: item.id });
        break;
      case 'rename_property': if (prop) mark(prop, { kind: 'rename', from: prop.name, to: c.new_name }); break;
      case 'alter_property_type': if (prop) mark(prop, { kind: 'retype', from: prop.type, to: c.to }); break;
      case 'drop_property': if (prop) mark(prop, { kind: 'drop' }); break;
      case 'set_required': if (prop) mark(prop, { kind: 'required' }); break;
      case 'add_property': if (t) t.props.push({ name: c.property, type: c.data_type, marks: [{ kind: 'add', item: item.id }] }); break;
      case 'rename_type': if (t) { t.renamedTo = c.new_name; t.renameItem = item.id; } break;
      case 'add_parent': if (t) { t.parents.push(c.parent); t.addedParents.push(c.parent); } break;
      case 'remove_parent': if (t) t.removedParents.push(c.parent); break;
      default: break;
    }
  }
  return types;
}

// -------------------------------------------------------------------- map ---

const CARD_W = 268, GAP_X = 34, GAP_Y = 64;

function layoutTrees(types) {
  const ents = [...types.values()].filter((t) => t.kind !== 'r');
  const primary = (t) => t.parents.find((p) => types.has(p) && !t.removedParents.includes(p)) ||
                         t.parents.find((p) => types.has(p));
  const children = new Map();
  const roots = [];
  for (const t of ents) {
    const p = primary(t);
    if (p) { if (!children.has(p)) children.set(p, []); children.get(p).push(t.name); } else roots.push(t.name);
  }
  const width = new Map();
  const w = (n) => {
    if (width.has(n)) return width.get(n);
    const kids = children.get(n) || [];
    const v = Math.max(CARD_W, kids.reduce((a, k) => a + w(k), 0) + GAP_X * Math.max(0, kids.length - 1));
    width.set(n, v); return v;
  };
  const pos = new Map();
  const place = (n, x, depth) => {
    pos.set(n, { x: x + (w(n) - CARD_W) / 2, depth });
    let cx = x;
    for (const k of children.get(n) || []) { place(k, cx, depth + 1); cx += w(k) + GAP_X; }
  };
  // trees first; types with no subtypes are stacked in a column to the right
  const trees = roots.filter((r) => children.get(r)?.length).sort((a, b) => w(b) - w(a) || a.localeCompare(b));
  const lone = roots.filter((r) => !children.get(r)?.length).sort();
  let x = 0;
  for (const r of trees) { place(r, x, 0); x += w(r) + GAP_X * 1.6; }
  lone.forEach((r, i) => pos.set(r, { x: x + Math.floor(i / 3) * (CARD_W + GAP_X), depth: 0, stack: i % 3 }));
  return { pos, totalW: x };
}

function propRow(t, p) {
  const m = p.marks || [];
  const rn = m.find((x) => x.kind === 'rename');
  const rt = m.find((x) => x.kind === 'retype');
  const cls = ['prop', p.inherited ? 'inh' : '', m.some((x) => x.kind === 'drop') ? 'dropped' : '',
               m.some((x) => x.kind === 'add') ? 'added' : '', m.length ? 'staged' : ''].join(' ');
  const name = rn ? `<s>${escapeHtml(rn.from)}</s> → <b>${escapeHtml(rn.to)}</b>` : escapeHtml(p.name);
  const type = rt ? `<s>${escapeHtml(rt.from)}</s> → <b>${escapeHtml(rt.to)}</b>` : escapeHtml(p.type || '');
  return `<div class="${cls}" data-type="${escapeHtml(t.name)}" data-prop="${escapeHtml(p.name)}" title="${p.inherited ? 'inherited' : 'click for changes'}">
    <span class="pn">${name}${p.required || m.some((x) => x.kind === 'required') ? '<i>*</i>' : ''}</span>
    <span class="pt">${type}</span><span class="pc">${escapeHtml(p.column || '')}</span></div>`;
}

function card(t) {
  const own = t.props.filter((p) => !p.inherited);
  const inh = t.props.filter((p) => p.inherited);
  const title = t.renamedTo ? `<s>${escapeHtml(t.name)}</s> → ${escapeHtml(t.renamedTo)}` : escapeHtml(t.name);
  return `<div class="dcard ${t.isNew ? 'ghost' : ''} ${t.renamedTo ? 'staged' : ''}" data-card="${escapeHtml(t.name)}" draggable="${t.kind === 'r' ? 'false' : 'true'}">
    <header data-type-head="${escapeHtml(t.name)}" style="--c:${colorFor(t.name)}">
      <span class="dot"></span><span class="tn">${title}</span>
      ${t.temporal ? '<span class="bdg tmp" title="temporal — history kept">⏱</span>' : ''}
      ${t.isNew ? '<span class="bdg new">new</span>' : `<span class="bdg">${Number(t.count).toLocaleString()}</span>`}
    </header>
    <div class="props">${own.map((p) => propRow(t, p)).join('') || '<div class="prop empty">no own properties</div>'}</div>
    ${inh.length ? `<div class="inh-line">+ ${inh.length} inherited · ${escapeHtml(inh.map((p) => p.name).join(', '))}</div>` : ''}
  </div>`;
}

function renderMap() {
  const host = $('#dz-canvas');
  if (!host || !D.snap) return;
  const types = virtualSchema();
  const { pos } = layoutTrees(types);
  const ents = [...types.values()].filter((t) => t.kind !== 'r');
  const rels = [...types.values()].filter((t) => t.kind === 'r');
  host.innerHTML = `<svg id="dz-edges"></svg>` +
    ents.map((t) => card(t)).join('') +
    `<div id="dz-rels"><span class="lbl">Relationship types</span>${rels.map((t) => card(t)).join('')}</div>`;
  // place cards: x from layout, y from the tallest card of each depth above
  const els = new Map($$('#dz-canvas > .dcard').map((el) => [el.dataset.card, el]));
  const depthH = [];
  for (const [n, p] of pos) if (p.stack == null) depthH[p.depth] = Math.max(depthH[p.depth] || 0, els.get(n)?.offsetHeight || 0);
  const top = (d) => 20 + depthH.slice(0, d).reduce((a, h) => a + h + GAP_Y, 0);
  const stackY = new Map();
  let maxY = 0, maxX = 0;
  for (const [n, p] of pos) {
    const el = els.get(n); if (!el) continue;
    let y = top(p.depth);
    if (p.stack != null) { y = stackY.get(p.x) ?? 20; stackY.set(p.x, y + el.offsetHeight + 22); }
    el.style.left = `${20 + p.x}px`; el.style.top = `${y}px`;
    maxY = Math.max(maxY, y + el.offsetHeight); maxX = Math.max(maxX, 20 + p.x + CARD_W);
  }
  const relsEl = $('#dz-rels'); relsEl.style.top = `${maxY + 40}px`;
  host.style.height = `${maxY + 60 + relsEl.offsetHeight}px`;
  host.style.minWidth = `${maxX + 40}px`;
  // edges parent → child (current solid, staged-added dashed, staged-removed red)
  const svg = $('#dz-edges'); svg.setAttribute('width', maxX + 40); svg.setAttribute('height', maxY + 20);
  const lines = [];
  for (const t of ents) {
    const ce = els.get(t.name); if (!ce) continue;
    for (const p of t.parents) {
      const pe = els.get(p); if (!pe) continue;
      const x1 = pe.offsetLeft + pe.offsetWidth / 2, y1 = pe.offsetTop + pe.offsetHeight;
      const x2 = ce.offsetLeft + ce.offsetWidth / 2, y2 = ce.offsetTop;
      const cls = t.removedParents.includes(p) ? 'rm' : t.addedParents.includes(p) || t.isNew ? 'add' : '';
      const my = (y1 + y2) / 2;
      lines.push(`<path class="${cls}" d="M${x1},${y1} C${x1},${my} ${x2},${my} ${x2},${y2}"/>`);
    }
  }
  svg.innerHTML = lines.join('');
  $('#dz-version').textContent = `schema v${D.snap.version}`;
}

async function loadDesigner() {
  const [snap, sch] = await Promise.all([
    api(`/api/schema-snapshot?graph=${encodeURIComponent(state.graph)}`, null, 'GET'),
    api(`/api/schema?graph=${encodeURIComponent(state.graph)}`, null, 'GET'),
  ]);
  if (!snap.ok) return;
  D.snap = snap.data;
  D.stats = new Map((sch.data?.stats?.types || []).map((t) => [t.name, t.instances]));
  $('#dz-graph').textContent = state.graph;
  renderMap();
  renderPlan();
}

// ------------------------------------------------------------- staging ---

function describe(c) {
  switch (c.kind) {
    case 'create_type': return `New type <b>${escapeHtml(c.type)}</b>${c.parents?.length ? ` under ${escapeHtml(c.parents.join(', '))}` : ''}`;
    case 'rename_property': return `Rename <b>${escapeHtml(c.type)}.${escapeHtml(c.property)}</b> → <b>${escapeHtml(c.new_name)}</b>`;
    case 'rename_type': return `Rename type <b>${escapeHtml(c.type)}</b> → <b>${escapeHtml(c.new_name)}</b>`;
    case 'add_parent': return `<b>${escapeHtml(c.type)}</b> also becomes a <b>${escapeHtml(c.parent)}</b>`;
    case 'remove_parent': return `<b>${escapeHtml(c.type)}</b> no longer extends <b>${escapeHtml(c.parent)}</b>`;
    case 'drop_property': return `Drop <b>${escapeHtml(c.type)}.${escapeHtml(c.property)}</b>`;
    case 'alter_property_type': return `Change <b>${escapeHtml(c.type)}.${escapeHtml(c.property)}</b> to <b>${escapeHtml(c.to)}</b>`;
    case 'set_required': return `Make <b>${escapeHtml(c.type)}.${escapeHtml(c.property)}</b> required`;
    case 'add_property': return `Add <b>${escapeHtml(c.type)}.${escapeHtml(c.property)}</b> : ${escapeHtml(c.data_type)}`;
    default: return escapeHtml(JSON.stringify(c));
  }
}

const REWRITES = new Set(['alter_property_type', 'set_required', 'add_property']);

function stage(change, source = 'map', problem = null) {
  D.plan.push({ id: ++D.seq, change, source, status: problem ? 'invalid' : 'staged', problem, phases: [] });
  renderPlan(); renderMap();
}

function renderPlan() {
  const list = $('#dz-plan');
  if (!list) return;
  list.innerHTML = D.plan.map((it) => {
    const st = it.status;
    const phaseChips = it.phases.map((p) => `<span class="ph ${p.bad ? 'bad' : ''}">${escapeHtml(p.name)}<small>${p.ms}ms</small></span>`).join('');
    const prog = it.progress != null ? `<div class="bar"><i style="width:${Math.round(it.progress * 100)}%"></i></div>` : '';
    const impact = it.impact ? `<div class="imp">${escapeHtml(it.impact)}</div>` : '';
    const result = st === 'done'
      ? `<div class="res ${it.storage?.written ? 'some' : 'zero'}">${it.total_ms} ms · ${it.storage ? `${Number(it.storage.written).toLocaleString()} / ${Number(it.storage.rows).toLocaleString()} rows rewritten` : 'catalog'}</div>` : '';
    const blocked = st === 'blocked'
      ? `<div class="res bad">validation found ${it.violations} violation(s)${it.samples?.length ? ` · ids ${escapeHtml(it.samples.slice(0, 3).join(', '))}` : ''}</div>
         <div class="acts"><button class="mini" data-retry="${it.id}">Retry validate</button><button class="mini" data-abort="${it.id}">Abort</button></div>` : '';
    return `<div class="pitem ${st}" data-item="${it.id}">
      <div class="ptop"><span class="kind ${REWRITES.has(it.change.kind) ? 'rw' : ''}">${escapeHtml(it.change.kind.replace(/_/g, ' '))}</span>
        ${it.source === 'ai' ? '<span class="src">AI</span>' : ''}
        <span class="pst">${escapeHtml(st)}</span>
        ${['staged', 'reviewed', 'invalid', 'failed'].includes(st) ? `<button class="x" data-rm="${it.id}" title="remove">✕</button>` : ''}</div>
      <div class="pdesc">${describe(it.change)}</div>
      ${it.problem ? `<div class="res bad">${escapeHtml(it.problem)}</div>` : ''}
      ${impact}${phaseChips ? `<div class="phs">${phaseChips}</div>` : ''}${prog}${result}${blocked}
    </div>`;
  }).join('') || '<div class="pane">Nothing staged. Click a property or a type on the map, drag a card onto a new parent, or ask in plain words above.</div>';
  const runnable = D.plan.some((it) => ['staged', 'reviewed'].includes(it.status));
  $('#dz-review').disabled = !runnable || D.running;
  $('#dz-run').disabled = !runnable || D.running;
}

// ------------------------------------------------------------- running ---

async function evoCall(action, item, extra = {}) {
  const t0 = performance.now();
  const { ok, data } = await api('/api/schema-change', {
    graph: state.graph, action, change: item.change, change_id: item.change_id, ...extra,
  });
  if (!ok) throw new Error(data.message || data.error || 'failed');
  const ms = data.elapsed_ms ?? Math.round(performance.now() - t0);
  return { data, ms };
}

async function review(item) {
  if (item.change.kind === 'create_type') { item.impact = 'catalog insert — new type, no rows'; item.status = 'reviewed'; return; }
  try {
    const { data, ms } = await evoCall('plan', item);
    item.change_id = data.change_id;
    item.phases = [{ name: 'plan', ms }];
    const r = data.report || {};
    const bits = [];
    if (data.storage) bits.push(`${Number(data.storage.rows).toLocaleString()} rows in scope`);
    bits.push(REWRITES.has(item.change.kind) ? 'rewrites rows in batches' : 'catalog only — no row rewritten');
    if (r.uncastable) bits.push(`${r.uncastable} value(s) will not convert`);
    if (r.lock) bits.push(r.lock);
    item.impact = bits.join(' · ');
    item.status = 'reviewed';
  } catch (e) {
    item.status = 'staged';
    item.impact = /does not exist/.test(e.message)
      ? 'depends on an earlier change in this plan — reviewed when it runs'
      : `planned at run time (${e.message.slice(0, 80)})`;
  }
}

async function reviewAll() {
  D.running = true; renderPlan();
  for (const it of D.plan) if (it.status === 'staged') { await review(it); renderPlan(); }
  D.running = false; renderPlan();
}

async function runItem(it) {
  const t0 = performance.now();
  it.status = 'running'; renderPlan();
  if (it.change.kind === 'create_type') {
    const { ok, data } = await api('/api/schema-type', { graph: state.graph, name: it.change.type, parents: it.change.parents || [] });
    if (!ok) throw new Error(data.message || data.error);
    it.phases = [{ name: 'create', ms: data.elapsed_ms }];
    it.total_ms = data.elapsed_ms; it.status = 'done';
    return;
  }
  if (!it.change_id) { const { data, ms } = await evoCall('plan', it); it.change_id = data.change_id; it.phases = [{ name: 'plan', ms }]; }
  renderPlan();
  let r = await evoCall('expand', it); it.phases.push({ name: 'expand', ms: r.ms }); renderPlan();
  if (REWRITES.has(it.change.kind)) {
    let total = 0;
    for (;;) {
      r = await evoCall('migrate', it, { batch: Number($('#dz-batch').value) || 100000 });
      total += r.ms;
      const rows = r.data.storage?.rows || 1;
      it.progress = Math.min(1, (r.data.report?.migrated || 0) / rows);
      it.phases = it.phases.filter((p) => p.name !== 'migrate').concat([{ name: 'migrate', ms: total }]);
      renderPlan();
      if (r.data.report?.done) break;
    }
    it.progress = null;
  }
  if (!(await validateAndContract(it))) return;
  it.total_ms = Math.round(it.phases.reduce((a, p) => a + p.ms, 0));
  void t0;
}

async function validateAndContract(it) {
  let r = await evoCall('validate', it);
  if (r.data.report?.violations) {
    it.phases.push({ name: 'validate', ms: r.ms, bad: true });
    it.status = 'blocked'; it.violations = r.data.report.violations; it.samples = r.data.report.samples || [];
    renderPlan();
    return false;
  }
  it.phases = it.phases.filter((p) => !(p.name === 'validate' && p.bad)).concat([{ name: 'validate', ms: r.ms }]);
  r = await evoCall('contract', it);
  it.phases.push({ name: 'contract', ms: r.ms });
  it.storage = r.data.storage; it.status = 'done';
  it.total_ms = Math.round(it.phases.reduce((a, p) => a + p.ms, 0));
  return true;
}

async function runAll() {
  D.running = true; renderPlan();
  try {
    for (const it of D.plan) {
      if (!['staged', 'reviewed'].includes(it.status)) continue;
      try {
        await runItem(it);
      } catch (e) {
        it.status = 'failed'; it.problem = e.message; renderPlan(); break;
      }
      renderPlan();
      if (it.status === 'blocked') break;
      await loadDesigner();
    }
  } finally {
    D.running = false; renderPlan(); loadEvo(); loadSchema();
  }
}

// ------------------------------------------------------------- AI assist ---

async function askAi() {
  const prompt = $('#dz-prompt').value.trim();
  if (!prompt) return;
  const out = $('#dz-ai-out');
  out.className = 'ai-out busy'; out.textContent = 'thinking…';
  $('#dz-ask').disabled = true;
  const { ok, data } = await api('/api/schema-assist', { graph: state.graph, prompt });
  $('#dz-ask').disabled = false;
  if (!ok) { out.className = 'ai-out bad'; out.textContent = data.error || data.message || 'failed'; return; }
  const okN = data.changes.filter((c) => c.ok).length;
  out.className = 'ai-out';
  out.innerHTML = `<div>${escapeHtml(data.explanation)}</div><div class="meta">${okN} of ${data.changes.length} proposals pass the catalog check · ${escapeHtml(data.model)} · ${data.elapsed_ms} ms — review below before running</div>`;
  for (const c of data.changes) stage(c.change, 'ai', c.ok ? null : c.problem);
}

// ------------------------------------------------------- map interactions ---

function closeMenu() { $('#dz-menu')?.remove(); }

function menu(x, y, title, items) {
  closeMenu();
  const m = document.createElement('div');
  m.id = 'dz-menu';
  m.innerHTML = `<div class="mt">${title}</div>` + items.map((it, i) =>
    it.input ? `<div class="mi in"><input data-i="${i}" placeholder="${escapeHtml(it.placeholder || '')}" value="${escapeHtml(it.value || '')}"><button class="mini" data-go="${i}">${escapeHtml(it.label)}</button></div>`
      : it.sub ? `<div class="mi sub"><span>${escapeHtml(it.label)}</span>${it.sub.map((s, j) => `<button class="mini" data-sub="${i}:${j}">${escapeHtml(s)}</button>`).join('')}</div>`
        : `<div class="mi" data-go="${i}">${escapeHtml(it.label)}</div>`).join('');
  document.body.appendChild(m);
  m.style.left = `${Math.min(x, window.innerWidth - 300)}px`; m.style.top = `${Math.min(y, window.innerHeight - m.offsetHeight - 10)}px`;
  m.onclick = (e) => {
    const go = e.target.closest('[data-go]');
    const sub = e.target.closest('[data-sub]');
    if (go) {
      const it = items[Number(go.dataset.go)];
      const val = it.input ? m.querySelector(`input[data-i="${go.dataset.go}"]`).value.trim() : null;
      if (it.input && !val) return;
      it.act(val); closeMenu();
    } else if (sub) {
      const [i, j] = sub.dataset.sub.split(':').map(Number);
      items[i].act(items[i].sub[j]); closeMenu();
    }
  };
  m.addEventListener('keydown', (e) => { if (e.key === 'Enter') m.querySelector('button[data-go]')?.click(); if (e.key === 'Escape') closeMenu(); });
  m.querySelector('input')?.focus();
}

function onMapClick(e) {
  const prop = e.target.closest('.prop[data-prop]');
  const head = e.target.closest('[data-type-head]');
  if (prop && !prop.classList.contains('inh')) {
    const type = prop.dataset.type, p = prop.dataset.prop;
    menu(e.clientX, e.clientY, `${type}.${p}`, [
      { label: 'Rename', input: true, placeholder: 'new name', act: (v) => stage({ kind: 'rename_property', type, property: p, new_name: v }) },
      { label: 'Change type', sub: TYPE_CHOICES, act: (v) => stage({ kind: 'alter_property_type', type, property: p, to: v }) },
      { label: 'Make required', act: () => stage({ kind: 'set_required', type, property: p }) },
      { label: 'Drop property', act: () => stage({ kind: 'drop_property', type, property: p }) },
    ]);
    return;
  }
  if (head) {
    const type = head.dataset.typeHead;
    const t = D.snap.types.find((x) => x.name === type);
    const others = D.snap.types.filter((x) => x.kind === t?.kind && x.name !== type).map((x) => x.name);
    const items = [
      { label: 'Rename type', input: true, placeholder: 'new name', act: (v) => stage({ kind: 'rename_type', type, new_name: v }) },
      { label: 'Add property', input: true, placeholder: 'name:type  e.g. rating:int', act: (v) => {
        const [property, data_type = 'string'] = v.split(':').map((s) => s.trim());
        stage({ kind: 'add_property', type, property, data_type });
      } },
      { label: 'New subtype', input: true, placeholder: 'type name', act: (v) => stage({ kind: 'create_type', type: v, parents: [type] }) },
      { label: 'Insert type above', input: true, placeholder: 'type name', act: (v) => {
        const old = t?.parents || [];
        stage({ kind: 'create_type', type: v, parents: old });
        stage({ kind: 'add_parent', type, parent: v });
        for (const p of old) stage({ kind: 'remove_parent', type, parent: p });
      } },
    ];
    if (t?.parents?.length) items.push({ label: 'Remove parent', sub: t.parents, act: (v) => stage({ kind: 'remove_parent', type, parent: v }) });
    if (others.length) items.push({ label: 'Add parent', sub: others.slice(0, 8), act: (v) => stage({ kind: 'add_parent', type, parent: v }) });
    menu(e.clientX, e.clientY, type, items);
    return;
  }
  if (!e.target.closest('#dz-menu')) closeMenu();
}

let dragType = null;
function onDragStart(e) { const c = e.target.closest('[data-card]'); if (c) { dragType = c.dataset.card; e.dataTransfer.setData('text/plain', dragType); c.classList.add('dragging'); } }
function onDragOver(e) { const c = e.target.closest('#dz-canvas > [data-card]'); if (c && dragType && c.dataset.card !== dragType) { e.preventDefault(); $$('.dcard.drop').forEach((x) => x.classList.remove('drop')); c.classList.add('drop'); } }
function onDrop(e) {
  const c = e.target.closest('[data-card]');
  $$('.dcard.drop, .dcard.dragging').forEach((x) => x.classList.remove('drop', 'dragging'));
  if (!c || !dragType || c.dataset.card === dragType) return;
  e.preventDefault();
  const child = dragType, parent = c.dataset.card;
  const old = D.snap.types.find((x) => x.name === child)?.parents || [];
  stage({ kind: 'add_parent', type: child, parent });
  for (const p of old) if (p !== parent) stage({ kind: 'remove_parent', type: child, parent: p });
  dragType = null;
}

// --------------------------------------------------------------- wiring ---

function openDesigner(on) {
  D.open = on;
  document.body.classList.toggle('designing', on);
  if (on) loadDesigner();
  else closeMenu();
}

$('#dz-canvas').addEventListener('click', onMapClick);
$('#dz-canvas').addEventListener('dragstart', onDragStart);
$('#dz-canvas').addEventListener('dragover', onDragOver);
$('#dz-canvas').addEventListener('drop', onDrop);
$('#dz-canvas').addEventListener('dragend', () => $$('.dcard.drop, .dcard.dragging').forEach((x) => x.classList.remove('drop', 'dragging')));
$('#dz-ask').onclick = askAi;
$('#dz-prompt').addEventListener('keydown', (e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) askAi(); });
$('#dz-review').onclick = reviewAll;
$('#dz-run').onclick = runAll;
$('#dz-clear').onclick = async () => {
  for (const it of D.plan) if (it.change_id && !['done'].includes(it.status)) await api('/api/schema-change', { graph: state.graph, action: 'abort', change_id: it.change_id }).catch(() => {});
  D.plan = D.plan.filter((it) => it.status === 'done' && false); renderPlan(); renderMap();
};
$('#dz-json-add').onclick = () => {
  try { const c = JSON.parse($('#dz-json').value); (Array.isArray(c) ? c : [c]).forEach((x) => stage(x, 'json')); }
  catch { alert('not valid JSON'); }
};
$('#dz-plan').addEventListener('click', async (e) => {
  const rm = e.target.closest('[data-rm]'), retry = e.target.closest('[data-retry]'), ab = e.target.closest('[data-abort]');
  const find = (el, k) => D.plan.find((x) => x.id === Number(el.dataset[k]));
  if (rm) {
    const it = find(rm, 'rm');
    if (it.change_id) await api('/api/schema-change', { graph: state.graph, action: 'abort', change_id: it.change_id });
    D.plan = D.plan.filter((x) => x !== it); renderPlan(); renderMap();
  }
  if (retry) {
    const it = find(retry, 'retry');
    D.running = true; it.status = 'running'; renderPlan();
    try { if (await validateAndContract(it)) { await loadDesigner(); loadEvo(); } } finally { D.running = false; renderPlan(); }
  }
  if (ab) {
    const it = find(ab, 'abort');
    await api('/api/schema-change', { graph: state.graph, action: 'abort', change_id: it.change_id });
    it.status = 'aborted'; renderPlan(); loadEvo();
  }
});
window.addEventListener('resize', () => D.open && renderMap());
