'use strict';
(() => {
  const $ = id => document.getElementById(id);
  const node = (tag, text, className) => { const n = document.createElement(tag); if (text !== undefined) n.textContent = text; if (className) n.className = className; return n; };
  let token = '', model = null, view = 'guided', editor = 'guided', preview = null, dirty = false, busy = false;
  const changes = new Set();
  const field = (key, label, help, type = 'text') => ({key, label, help, type});
  const groups = [
    ['Connections', 'Addresses and ports identify where visitors and administrators connect. Listener changes require a restart.', [
      field('listen', 'App address and port', '127.0.0.1 accepts this computer only. 0.0.0.0 accepts IPv4 connections from other computers; configure HTTPS and access rules before publishing.'),
      field('admin_listen', 'Private administration address', 'Must be a loopback IP address and a different port. Dashboard: / · Administration: /admin. This listener serves HTTP.'),
      field('http_allow_compression', 'Compress responses', 'Reduce transfer size for compressible HTTP responses. Streaming Shiny messages remain uncompressed.', 'checkbox'),
      field('max_inflight', 'Maximum simultaneous requests', 'Global capacity limit. More is not always faster; R worker limits are configured per app.', 'number')]],
    ['R and storage', 'Paths can be absolute or relative to the configuration file. Changing the state directory requires a restart and does not move existing data.', [
      field('rscript', 'R executable', 'Path to Rscript, or its command name. Use doctor to check R and installed Shiny packages.'),
      field('state_dir', 'Server data directory', 'Stores private tokens, usage statistics, worker logs and ACME certificates. Keep it outside served app folders.'),
      field('shutdown_grace_seconds', 'Shutdown grace (seconds)', 'Time allowed for active work to finish when the server stops.', 'number'),
      field('response_header_timeout_seconds', 'Response timeout (seconds)', 'How long the gateway waits for an app to begin an ordinary HTTP response.', 'number')]],
    ['Usage statistics', 'Settings affect collection and retention. The dashboard explains what each count measures.', [
      field('analytics.enabled', 'Collect usage statistics', 'Record aggregate request and connection activity. ShinyHarbor analytics does not retain raw visitor IP addresses.', 'checkbox'),
      field('analytics.retention_days', 'Keep usage data (days)', 'Older analytics are pruned. Reducing this value may remove older data after restart.', 'number'),
      field('analytics.queue_capacity', 'Analytics queue capacity', 'Bounded in-memory queue. Events can be dropped under overload rather than holding up app traffic.', 'number')]],
    ['Connections and recovery', 'These global transport settings require a restart. Most installations should keep their defaults.', [
      field('http_keepalive_timeout_seconds', 'Idle HTTP connection timeout', 'Seconds to keep idle HTTP connections open. Zero disables the timeout.', 'number'),
      field('sockjs_heartbeat_seconds', 'Heartbeat interval (seconds)', 'Keeps supported Shiny browser transports active.', 'number'),
      field('sockjs_disconnect_seconds', 'Reconnect grace (seconds)', 'Time before an abandoned transport session is closed.', 'number'),
      field('sockjs.memory_budget_bytes', 'Recovery memory budget (bytes)', 'A global cap for queued recovery data; avoids unbounded buffering.', 'number')]]
  ];
  const valueAt = (value, key) => key.split('.').reduce((v, k) => v?.[k], value);
  async function api(body) {
    const response = await fetch('/api/config', {method: body ? 'POST' : 'GET', cache: 'no-store', headers: {'Authorization': `Bearer ${token}`, ...(body ? {'Content-Type':'application/json'} : {})}, ...(body ? {body:JSON.stringify(body)} : {})});
    let result; try { result = await response.json(); } catch { throw Error(`The server returned HTTP ${response.status}.`); }
    if (!response.ok) throw Error(result.error || `HTTP ${response.status}`);
    return result;
  }
  function invalidate() { dirty = true; preview = null; $('save').disabled = true; $('preview').hidden = true; $('validation').textContent = 'Unsaved edits. Validate them before saving.'; }
  function setBusy(value) { busy = value; for (const id of ['validate','refresh','discard']) $(id).disabled = value; $('save').disabled = value || !preview; }
  function control(f, value, parent, key = f.key) {
    const box = node('div', undefined, 'setting'), id = `setting-${key.replaceAll('.','-')}`;
    const label = node('label', f.label); label.htmlFor = id; box.append(label);
    const input = node('input'); input.id = id; input.type = f.type; input.dataset.key = key;
    if (f.type === 'checkbox') input.checked = !!value; else { input.value = value ?? ''; if (f.type === 'number') input.step = 'any'; }
    input.addEventListener('input', () => { changes.add(key); invalidate(); });
    box.append(input, node('small', f.help), node('code', key)); parent.append(box); return input;
  }
  function tlsFields() {
    const section = node('section', undefined, 'card settings-group'); section.append(node('h2', 'HTTPS certificates'), node('p', 'Public domains can use automatic ACME certificates. Localhost needs a locally trusted certificate in PEM mode. This page does not install browser trust or open firewall ports.'));
    const grid = node('div', undefined, 'settings-grid'), box = node('div', undefined, 'setting');
    const label = node('label','Certificate mode'); label.htmlFor = 'tls-mode'; box.append(label);
    const select = node('select'); select.id = 'tls-mode';
    for (const [v,t] of [['off','HTTP only'],['pem','Use certificate files (PEM)'],['acme','Automatic certificates (public domain)']]) { const option = node('option',t); option.value = v; select.append(option); }
    select.value = model.configured?.tls?.mode || 'off'; box.append(select); grid.append(box);
    const fields = node('div', undefined, 'settings-grid');
    function update() {
      fields.replaceChildren(); const tls = model.configured?.tls || {};
      if (select.value === 'pem') {
        control(field('tls.cert','Certificate file','PEM certificate/full chain. Relative paths start beside the configuration file.'), tls.cert || 'certs/localhost.pem', fields);
        control(field('tls.key','Private key file','Keep this file private and outside served folders. The service account must be able to read it.'), tls.key || 'certs/localhost-key.pem', fields);
      } else if (select.value === 'acme') {
        control(field('tls.domains','Domain names','Comma-separated public domains pointing to this server. Localhost and wildcard names are not supported.'), (tls.domains || []).join(', '), fields);
        control(field('tls.email','Contact email','An address for the certificate account.'), tls.email || '', fields);
        control(field('tls.production','Use trusted production certificates','Leave unchecked for the ACME staging service. Staging certificates are not trusted by browsers.', 'checkbox'), tls.production || false, fields);
      }
    }
    select.addEventListener('change', () => { update(); changes.add('tls'); invalidate(); }); update();
    section.append(grid, fields); $('fields').prepend(section);
  }
  function render() {
    $('config-path').textContent = model.path; $('pending').textContent = model.pending ? 'Saved settings differ from running settings' : 'Saved settings match the running server';
    $('source').value = model.source; $('fields').replaceChildren();
    $('legacy-note').hidden = model.format !== 'legacy'; $('fields').hidden = model.format === 'legacy' || !model.configured;
    for (const [title, help, fields] of groups) {
      const section = node('section', undefined, 'card settings-group'); section.append(node('h2',title), node('p',help));
      const grid = node('div', undefined, 'settings-grid'); for (const f of fields) control(f, valueAt(model.configured,f.key), grid); section.append(grid); $('fields').append(section);
    }
    tlsFields(); activeValues();
    if (model.format === 'legacy' || !model.configured) show('source');
  }
  function activeValues() {
    const query = $('filter').value.toLowerCase(); $('active-values').replaceChildren();
    function walk(value, path) {
      if (value && typeof value === 'object' && Object.keys(value).length) { for (const [key, child] of Object.entries(value)) walk(child, path ? `${path}.${key}` : key); return; }
      const text = JSON.stringify(value); if (!`${path} ${text}`.toLowerCase().includes(query)) return;
      const row = node('div', undefined, 'active-row'); row.append(node('code',path), node('pre',text)); $('active-values').append(row);
    }
    walk(model.active, '');
  }
  function show(next) {
    view = next;
    if (['guided','source'].includes(next)) editor = next;
    for (const id of ['guided','source','active','help']) { $(`view-${id}`).hidden = id !== next; $(`tab-${id}`).setAttribute('aria-pressed', String(id === next)); }
    $('edit-actions').hidden = !['guided','source'].includes(next);
  }
  function draft() {
    const body = {revision:model.revision};
    if (view === 'source') body.source = $('source').value;
    else {
      body.changes = {};
      for (const key of changes) {
        if (key.startsWith('tls')) continue;
        const input = $(`setting-${key.replaceAll('.','-')}`);
        if (input.type === 'number' && (input.value.trim() === '' || !Number.isFinite(Number(input.value)))) throw Error(`Enter a number for ${key}.`);
        body.changes[key] = input.type === 'checkbox' ? input.checked : input.type === 'number' ? Number(input.value) : input.value;
      }
      if ([...changes].some(k => k.startsWith('tls'))) {
        const mode = $('tls-mode').value, tls = {mode};
        if (mode === 'pem') { tls.cert = $('setting-tls-cert').value; tls.key = $('setting-tls-key').value; }
        if (mode === 'acme') { tls.domains = $('setting-tls-domains').value.split(',').map(s=>s.trim()).filter(Boolean); tls.email = $('setting-tls-email').value; tls.production = $('setting-tls-production').checked;
          // Preserve custom CA settings when editing the same ACME block.
          for (const k of ['directory_url','ca_bundle']) if (model.configured?.tls?.[k]) tls[k] = model.configured.tls[k];
        }
        body.changes.tls = tls;
      }
    }
    return body;
  }
  async function load() {
    model = await api(); changes.clear(); dirty = false; preview = null; render();
    $('content').hidden = false; $('login').hidden = true; $('lock').hidden = false; $('refresh').disabled = false;
    $('preview').hidden = true; $('save').disabled = true;
    $('restart-help').hidden = !model.pending;
    $('validation').textContent = 'Validate before saving. Nothing changes on the server until you save.';
  }
  async function action(fn) { if (busy) return; setBusy(true); $('message').textContent = ''; try { await fn(); } catch (e) { $('message').textContent = e.message; } finally { setBusy(false); } }
  $('unlock').addEventListener('click', () => { token = $('token').value.trim(); $('token').value = ''; action(load); });
  $('token').addEventListener('keydown', e => { if (e.key === 'Enter') $('unlock').click(); });
  $('lock').addEventListener('click', () => { if (dirty && !confirm('Discard unsaved edits and lock administration?')) return; location.reload(); });
  for (const id of ['refresh','discard']) $(id).addEventListener('click', () => { if (!dirty || confirm('Discard your unsaved edits and reload the file?')) action(load); });
  $('source').addEventListener('input', invalidate); $('filter').addEventListener('input', activeValues);
  document.querySelectorAll('[data-view]').forEach(button => button.addEventListener('click', () => {
    if (busy) return;
    const next = button.dataset.view;
    if (['guided','source'].includes(next) && next !== editor && dirty) { $('message').textContent = 'Save or discard your edits before switching editors. You can still view running settings and the reference.'; return; }
    show(next);
  }));
  $('validate').addEventListener('click', () => action(async () => {
    preview = await api(draft()); $('validation').textContent = preview.message;
    $('preview-source').textContent = preview.source; $('preview').hidden = false;
    $('save').textContent = preview.restart_required ? '2. Save for next restart' : '2. Save and apply';
  }));
  $('save').addEventListener('click', () => action(async () => {
    if (!preview) return;
    const result = await api({revision:model.revision, source:preview.source, save:true});
    await load(); $('message').textContent = `${result.message}\nBackup: ${result.backup}`;
    $('restart-help').hidden = !result.restart_required;
  }));
  window.addEventListener('beforeunload', e => { if (dirty) { e.preventDefault(); e.returnValue = ''; } });
  fetch('/admin/reference').then(r=>r.text()).then(text=>{ $('reference').textContent = text; }).catch(()=>{ $('reference').textContent = 'The reference could not be loaded. See docs/CONFIGURATION.md in your installation.'; });
})();
