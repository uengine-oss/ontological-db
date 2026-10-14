'use strict';
/**
 * Releases and timelines in the console.
 *
 *   :releases
 *       Releases applied to this graph — package hash, schema version before →
 *       after, how many schema changes and instance writes each one made.
 *
 *   :timeline <Type> <key_prop>=<key> [from] [to] [@system_at]
 *       One element's validity slices over a period, as bars on a time axis.
 *       With @system_at, as it was believed then — a correction recorded later
 *       is not there yet.
 *       e.g.  :timeline Clause clause_key=DOC-RP-SUPV#제9조 2026-10-01 2027-12-31
 *
 * Both read the management API (GET /api/releases, GET /api/timeline), which
 * asks the database (og_releases, og_timeline).
 */

async function runReleasesCmd(frame) {
  const { ok, data } = await api(`/api/releases?graph=${encodeURIComponent(state.graph)}`, null, 'GET');
  if (!ok) { setViews(frame, [{ name: 'Error', node: errorView(data, ':releases') }], '<span class="bad">failed</span>'); return; }
  const rows = data.map((r) => ({
    '#': r.release_id, name: r.name, status: r.status,
    schema: `v${r.from_version} → v${r.to_version}`,
    changes: (r.change_ids || []).length, writes: r.instances,
    actor: r.actor || r.created_by, finished: (r.finished_at || '').replace('T', ' ').slice(0, 19),
    package: (r.package_hash || '').slice(0, 19),
  }));
  setViews(frame, [{ name: 'Table', node: tableView(rows, Object.keys(rows[0] || { '#': 0 })) },
                   { name: 'JSON', node: codeView(JSON.stringify(data, null, 2)) }],
    `<span class="ok">${data.length} releases</span><span>og_releases</span>`);
}

function parseTimelineArgs(arg) {
  const parts = arg.trim().split(/\s+/);
  const [type, kv, ...rest] = parts;
  const eq = (kv || '').indexOf('=');
  if (!type || eq < 1) return null;
  const out = { type, key_prop: kv.slice(0, eq), key: kv.slice(eq + 1) };
  const dates = [];
  for (const r of rest) {
    if (r.startsWith('@')) out.system_at = r.slice(1);
    else dates.push(r);
  }
  if (dates[0]) out.from = dates[0];
  if (dates[1]) out.to = dates[1];
  return out;
}

const TL_DAY = 86400000;
const tlDate = (s) => (s ? s.slice(0, 10) : '∞');

/**
 * What the bars are about: the properties whose value is not the same in every
 * slice. Long text is marked, not printed — the Table view has it.
 */
function varyingProps(slices) {
  const keys = new Set(slices.flatMap((s) => Object.keys(s.props || {})));
  return [...keys].filter((k) => new Set(slices.map((s) => JSON.stringify((s.props || {})[k]))).size > 1);
}
function propLabel(k, v) {
  const t = String(v ?? '');
  return t.length > 24 ? `${k} ✎` : `${k}=${t}`;
}

function timelineView(slices, args) {
  const box = document.createElement('div');
  box.className = 'tl';
  if (!slices.length) { box.innerHTML = '<div class="tl-empty">이 기간에 유효한 값이 없습니다</div>'; return box; }
  const t = (s) => (s ? Date.parse(s) : null);
  const lo = args.from ? Date.parse(args.from) : Math.min(...slices.map((s) => t(s.valid_from) ?? Date.now()));
  const hiCand = slices.map((s) => t(s.valid_to)).filter(Boolean);
  const hi = args.to ? Date.parse(args.to) : Math.max(Date.now() + 365 * TL_DAY, ...hiCand);
  const span = Math.max(hi - lo, TL_DAY);
  const x = (ms) => Math.min(100, Math.max(0, ((ms - lo) / span) * 100));

  const axis = document.createElement('div');
  axis.className = 'tl-axis';
  // month ticks (at most ~14)
  const d0 = new Date(lo); d0.setUTCDate(1);
  const months = Math.max(1, Math.round(span / (30 * TL_DAY)));
  const step = Math.ceil(months / 14);
  for (let d = new Date(Date.UTC(d0.getUTCFullYear(), d0.getUTCMonth() + 1, 1)); d.getTime() < hi; d.setUTCMonth(d.getUTCMonth() + step)) {
    const tick = document.createElement('span');
    tick.style.left = x(d.getTime()) + '%';
    tick.textContent = `${String(d.getUTCFullYear()).slice(2)}.${String(d.getUTCMonth() + 1).padStart(2, '0')}`;
    axis.appendChild(tick);
  }
  box.appendChild(axis);

  const vary = varyingProps(slices);
  let prev = null;
  slices.forEach((s, i) => {
    const row = document.createElement('div');
    row.className = 'tl-row';
    const a = t(s.valid_from) ?? lo, b = t(s.valid_to) ?? hi;
    const bar = document.createElement('div');
    bar.className = `tl-bar tl-c${i % 4}` + (s.op === 'current' ? ' tl-cur' : '');
    bar.style.left = x(a) + '%';
    bar.style.width = Math.max(0.8, x(b) - x(a)) + '%';
    const keys = vary.filter((k) => k !== 'amended_on' && (!prev || JSON.stringify(prev[k]) !== JSON.stringify((s.props || {})[k]) || !/text/.test(k)));
    const label = keys.map((k) => propLabel(k, (s.props || {})[k])).join(' · ');
    bar.innerHTML = `<b>${escapeHtml(tlDate(s.valid_from))} ~ ${escapeHtml(tlDate(s.valid_to))}</b>`;
    bar.title = `${s.source || ''}\n기록 ${s.recorded_at || ''}${s.superseded_at ? '\n대체됨 ' + s.superseded_at : ''}\n${JSON.stringify(s.props, null, 1)}`;
    const meta = document.createElement('div');
    meta.className = 'tl-meta';
    meta.innerHTML = `<b>${escapeHtml(label)}</b>${label ? ' · ' : ''}${escapeHtml(s.source || '')}`;
    row.appendChild(bar);
    row.appendChild(meta);
    box.appendChild(row);
    prev = s.props || {};
  });
  return box;
}

async function runTimelineCmd(frame, arg) {
  const args = parseTimelineArgs(arg);
  if (!args) {
    setViews(frame, [{ name: 'Usage', node: codeView(':timeline <Type> <key_prop>=<key> [from] [to] [@system_at]\n예) :timeline Clause clause_key=DOC-RP-SUPV#제9조 2026-10-01 2027-12-31') }],
      '<span class="bad">usage</span>');
    return;
  }
  const q = new URLSearchParams({ graph: state.graph, ...args });
  const { ok, data } = await api(`/api/timeline?${q}`, null, 'GET');
  if (!ok) { setViews(frame, [{ name: 'Error', node: errorView(data, ':timeline') }], '<span class="bad">failed</span>'); return; }
  const rows = data.slices.map((s) => ({ valid_from: tlDate(s.valid_from), valid_to: tlDate(s.valid_to), op: s.op,
                                         source: s.source, ...Object.fromEntries(Object.entries(s.props || {}).filter(([k]) => k !== 'text')) }));
  const known = args.system_at ? ` · ${escapeHtml(args.system_at)} 에 알던 것` : '';
  setViews(frame, [{ name: 'Timeline', node: timelineView(data.slices, args) },
                   { name: 'Table', node: tableView(rows, Object.keys(rows[0] || { valid_from: 0 })) },
                   { name: 'JSON', node: codeView(JSON.stringify(data, null, 2)) }],
    `<span class="ok">${data.slices.length} slices</span><span>${escapeHtml(args.type)} ${escapeHtml(args.key)} · ${escapeHtml(args.from || '-∞')} ~ ${escapeHtml(args.to || '∞')}${known}</span>`);
}
