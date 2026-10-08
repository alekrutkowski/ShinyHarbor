/* Original, dependency-free dashboard. No remote resources or persistent token storage. */
'use strict';
(() => {
  const $ = id => document.getElementById(id);
  const count = value => new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(Number(value) || 0);
  const bytes = value => {
    let n = Number(value) || 0, unit = 0;
    while (n >= 1024 && unit < 4) { n /= 1024; unit++; }
    return `${n.toFixed(unit ? 1 : 0)} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][unit]}`;
  };
  const text = (id, value) => { $(id).textContent = value; };
  const svgNS = 'http://www.w3.org/2000/svg';
  const svg = (name, attrs) => {
    const node = document.createElementNS(svgNS, name);
    Object.entries(attrs).forEach(([k, v]) => node.setAttribute(k, String(v)));
    return node;
  };
  let token = '', timer = null, controller = null, snapshot = null, descending = true;
  const message = value => text('message', value);
  function lock() {
    token = ''; snapshot = null;
    if (controller) controller.abort();
    clearTimeout(timer);
    $('content').hidden = true; $('login').hidden = false;
    $('lock').hidden = true; $('refresh').disabled = true;
    $('token').value = ''; message(''); $('token').focus();
  }
  function renderChart(series, days, now) {
    const width = Math.max(260, $('chart').clientWidth || 1000);
    const height = Math.max(120, $('chart').clientHeight || 185), left = 42, bottom = height - 25;
    const start = Math.floor(now / 3600) - days * 24;
    const byHour = new Map(series.map(row => [row.hour, row.pages]));
    const points = Array.from({ length: days * 24 + 1 }, (_, i) => [start + i, byHour.get(start + i) || 0]);
    const max = Math.max(4, ...points.map(p => p[1]));
    const chart = svg('svg', { viewBox: `0 0 ${width} ${height}`, preserveAspectRatio: 'none', 'aria-hidden': 'true' });
    for (let i = 0; i < 5; i++) {
      const y = 8 + (bottom - 8) * i / 4;
      chart.append(svg('line', { x1: left, x2: width, y1: y, y2: y, class: 'grid-line' }));
      const label = svg('text', { x: left - 12, y: y + 4, 'text-anchor': 'end', class: 'chart-label' });
      label.textContent = count(max * (4 - i) / 4); chart.append(label);
    }
    const coordinates = points.map((p, i) => `${left + (width - left) * i / (points.length - 1)},${bottom - (bottom - 8) * p[1] / max}`);
    chart.append(svg('path', { d: `M${left},${bottom} L${coordinates.join(' L')} L${width},${bottom} Z`, class: 'chart-area' }));
    chart.append(svg('polyline', { points: coordinates.join(' '), class: 'chart-line', 'vector-effect': 'non-scaling-stroke' }));
    $('chart').replaceChildren(chart);
    const date = n => new Date(n * 1000).toLocaleString(undefined, { timeZone: 'UTC', month: 'short', day: 'numeric', hour: '2-digit' });
    text('chart-start', date(start * 3600)); text('chart-end', date(now));
    $('chart').setAttribute('aria-label', `Hourly page views over ${days} day(s). Total ${count(points.reduce((n, p) => n + p[1], 0))}.`);
  }
  function renderRank(id, rows, label) {
    const container = $(id); container.replaceChildren();
    const relevant = rows.filter(r => r.pages > 0).slice(0, 5);
    if (!relevant.length) { const empty = document.createElement('p'); empty.className = 'empty'; empty.textContent = 'No visits recorded yet.'; container.append(empty); return; }
    const max = Math.max(...relevant.map(r => r.pages));
    relevant.forEach(row => {
      const item = document.createElement('div'); item.className = 'rank';
      const heading = document.createElement('div'); heading.className = 'rank-title';
      const name = document.createElement('span'); name.textContent = label(row); name.title = name.textContent;
      const total = document.createElement('strong'); total.textContent = count(row.pages);
      heading.append(name, total);
      const bar = svg('svg', { viewBox: '0 0 100 4', preserveAspectRatio: 'none', 'aria-hidden': 'true' });
      bar.append(svg('rect', { x: 0, y: 0, width: 100, height: 4, rx: 2, class: 'rank-bg' }), svg('rect', { x: 0, y: 0, width: 100 * row.pages / max, height: 4, rx: 2, class: 'rank-bar' }));
      item.append(heading, bar); container.append(item);
    });
  }
  function renderApps(data) {
    const rows = new Map(data.apps.map(row => [row.app, { ...row }]));
    (data.pools || []).forEach(pool => rows.set(pool.name, { ...(rows.get(pool.name) || { app: pool.name }), pool }));
    const list = [...rows.values()].sort((a, b) => (descending ? -1 : 1) * ((a.pages || 0) - (b.pages || 0)) || a.app.localeCompare(b.app));
    $('app-rows').replaceChildren();
    list.forEach(row => {
      const tr = document.createElement('tr'); const name = document.createElement('td');
      name.textContent = row.pool?.label || row.app; const mount = document.createElement('small');
      mount.textContent = row.pool ? `${row.pool.mount} · ${row.pool.runtime === 'native-r' ? 'Native R' : 'Upstream gateway'}` : 'Not currently configured';
      name.append(mount); tr.append(name);
      [count(row.pages), count(row.visitor_days), count(row.websockets), bytes((row.bytes_in || 0) + (row.bytes_out || 0)), count(row.errors), `${count(row.pool?.active_websockets)} / ${count(row.pool?.workers)}`].forEach(value => {
        const cell = document.createElement('td'); cell.textContent = value; tr.append(cell);
      });
      $('app-rows').append(tr);
    });
    text('app-count', `${list.length} APPLICATION${list.length === 1 ? '' : 'S'}`);
    $('empty-apps').hidden = list.some(row => row.requests > 0);
  }
  function render(data) {
    const sum = field => data.apps.reduce((n, row) => n + (Number(row[field]) || 0), 0);
    text('pages', count(sum('pages'))); text('visitors', count(sum('visitor_days')));
    text('websockets', count(sum('websockets')));
    text('active', count((data.pools || []).reduce((n, row) => n + row.active_websockets, 0)));
    renderChart(data.series, data.days, data.generated_at); renderApps(data);
    const regionNames = typeof Intl.DisplayNames === 'function' ? new Intl.DisplayNames(undefined, { type: 'region' }) : null;
    renderRank('countries', data.countries, row => {
      if (row.code === 'ZZ') return 'Unknown country';
      try { return regionNames?.of(row.code) || row.code; } catch { return row.code; }
    });
    renderRank('referrers', data.referrers, row => row.origin || 'Direct / unknown');
    text('geo-note', data.country_database_loaded ? 'Estimated from a local country database. VPNs and proxies affect accuracy.' : 'No country database configured. Add analytics.country_database to enable local lookup.');
    text('updated', `Updated ${new Date(data.generated_at * 1000).toLocaleTimeString()}`);
    text('quality', `${count(data.dropped_events)} analytics events dropped since start`);
    text('live-status', data.enabled ? 'Connected · refreshes every 15 seconds' : 'Connected · analytics disabled');
    message(data.dropped_events ? 'Some analytics events were dropped under load. Counts are incomplete.' : data.enabled ? '' : 'Analytics collection is disabled. Previously retained records may still be displayed.');
  }
  async function refresh() {
    if (!token) return;
    if (controller) controller.abort();
    clearTimeout(timer); controller = new AbortController();
    const current = controller;
    $('refresh').disabled = true;
    try {
      const response = await fetch(`/api/stats?days=${encodeURIComponent($('days').value)}`, {
        headers: { Authorization: `Bearer ${token}`, Accept: 'application/json' }, cache: 'no-store', credentials: 'omit', signal: controller.signal
      });
      if (response.status === 401) { lock(); message('The token was not accepted. Check the token on the server.'); return; }
      if (!response.ok) throw new Error(`Analytics unavailable (HTTP ${response.status}).`);
      const data = await response.json();
      if (current !== controller) return;
      $('login').hidden = true; $('content').hidden = false; $('lock').hidden = false; $('token').value = '';
      snapshot = data; render(data);
    } catch (error) { if (error.name !== 'AbortError') { message(error.message || 'Cannot reach the admin server.'); text('live-status', 'Connection interrupted'); } }
    finally { if (current === controller) { $('refresh').disabled = !token; if (token) timer = setTimeout(refresh, 15000); } }
  }
  $('login-form').addEventListener('submit', event => { event.preventDefault(); token = $('token').value.trim(); refresh(); });
  $('lock').addEventListener('click', lock); $('refresh').addEventListener('click', refresh);
  $('days').addEventListener('change', refresh);
  $('sort-pages').addEventListener('click', () => { descending = !descending; text('sort-pages', `Page views ${descending ? '↓' : '↑'}`); if (snapshot) renderApps(snapshot); });
  window.addEventListener('resize', () => { if (snapshot && token && !$('content').hidden) renderChart(snapshot.series, snapshot.days, snapshot.generated_at); });
  window.addEventListener('pagehide', () => { token = ''; if (controller) controller.abort(); clearTimeout(timer); });
})();
