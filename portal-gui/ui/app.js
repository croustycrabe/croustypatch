// CroustyPatch desktop app center. Talks to the agent's local socket through three Tauri commands
// (list, install, uninstall); the agent identifies the user from the socket, never from this window.
import { ICONS } from './icons.js';

const FR = {
  title: 'Portail applicatif', host: 'Machine {h} · demandes au nom de votre compte', all: 'Toutes', search: 'Rechercher une application',
  refresh: 'Actualiser', present: 'Présent', partial: 'Partiel {p}/{t}', available: 'Disponible', working: 'En cours…',
  install: 'Installer', uninstall: 'Désinstaller', complete: 'Compléter', cancel: 'Annuler', update: 'Mettre à jour', update_avail: 'Mise à jour {v}', confirm_update: 'Mettre à jour {n} ?',
  confirm_install: 'Installer {n} ?', confirm_uninstall: 'Désinstaller {n} ?', confirm_complete: 'Compléter {n} ?',
  packages: 'Paquets installés par le gestionnaire du système :', command: 'L’administrateur a prévu cette commande, exécutée par l’agent :',
  check: 'Vérifié ensuite : {p} doit être utilisable par votre compte.', missing: 'Manquants : {m}',
  status: 'État et notifications', clear: 'Effacer', idle: 'Aucune opération en cours.',
  stage_queued: 'en file d’attente', stage_sent: 'transmise à l’agent', stage_running: 'en cours', stage_soaking: 'vérification',
  installing: 'Installation de {n}', removing: 'Désinstallation de {n}',
  ok_install: '{n} installé', ok_uninstall: '{n} désinstallé', failed: 'Échec pour {n} : {m}', cancelled: '{n} : demande annulée',
  sent: 'Demande envoyée pour {n}.', opened: 'Déjà présentes sur cette machine : {l}', none_present: 'Aucune application du catalogue n’est encore présente.',
  empty: 'Aucune application proposée pour cette machine.', no_match: 'Aucune application ne correspond.',
  unreachable: 'L’agent CroustyPatch ne répond pas sur cette machine ({e}).', command_badge: 'commande',
  tab_apps: 'Applications', tab_updates: 'Mises à jour', upd_title: 'Mises à jour de cet ordinateur', upd_sub: '{n} mise(s) à jour disponible(s){sec}', upd_sec: ' dont {s} de sécurité',
  upd_none: 'Cet ordinateur est à jour.', upd_check: 'Rechercher', upd_checking: 'Recherche…', upd_go: 'Mettre à jour ({n})', upd_checked: 'Dernière recherche : {d}',
  upd_managed: 'Les mises à jour de cet ordinateur sont gérées par votre administrateur : vous pouvez les voir ici, mais pas les lancer.',
  upd_reboot: 'Un redémarrage est nécessaire pour terminer les mises à jour.', upd_security: 'Sécurité',
  confirm_upd: 'Mettre à jour {n} paquet(s) ?', upd_body: 'L’agent les installe avec un point de restauration et revient en arrière en cas de problème. L’ordinateur peut demander un redémarrage ensuite.',
  upd_running: 'Mise à jour du système', ok_upd: 'Mises à jour installées', fail_upd: 'La mise à jour a échoué : {m}', upd_sent: 'Mise à jour demandée ({n} paquet(s)).',
  check_started: 'Recherche de mises à jour lancée…', check_done: 'Recherche terminée : {n} mise(s) à jour disponible(s).', upd_unavailable: 'Les mises à jour ne sont pas disponibles sur cet ordinateur ({e}).',
  theme_system: 'Thème : celui du système', theme_light: 'Thème : clair', theme_dark: 'Thème : sombre', lang_title: 'Langue : français (cliquer pour passer à l’anglais)',
};
const EN = {
  title: 'App center', host: 'Machine {h} · requests made as your account', all: 'All', search: 'Search an application',
  refresh: 'Refresh', present: 'Present', partial: 'Partial {p}/{t}', available: 'Available', working: 'Working…',
  install: 'Install', uninstall: 'Uninstall', complete: 'Complete', cancel: 'Cancel', update: 'Update', update_avail: 'Update {v}', confirm_update: 'Update {n}?',
  confirm_install: 'Install {n}?', confirm_uninstall: 'Uninstall {n}?', confirm_complete: 'Complete {n}?',
  packages: 'Packages installed by the system’s package manager:', command: 'Your administrator set up this command, run by the agent:',
  check: 'Checked afterwards: {p} must be usable by your account.', missing: 'Missing: {m}',
  status: 'Status and notifications', clear: 'Clear', idle: 'No operation in progress.',
  stage_queued: 'queued', stage_sent: 'handed to the agent', stage_running: 'running', stage_soaking: 'checking',
  installing: 'Installing {n}', removing: 'Removing {n}',
  ok_install: '{n} installed', ok_uninstall: '{n} uninstalled', failed: '{n} failed: {m}', cancelled: '{n}: request cancelled',
  sent: 'Request sent for {n}.', opened: 'Already on this machine: {l}', none_present: 'No catalog application is on this machine yet.',
  empty: 'No application offered for this machine.', no_match: 'No application matches.',
  unreachable: 'The CroustyPatch agent does not answer on this machine ({e}).', command_badge: 'command',
  tab_apps: 'Applications', tab_updates: 'Updates', upd_title: 'Updates for this computer', upd_sub: '{n} update(s) available{sec}', upd_sec: ', {s} of them security',
  upd_none: 'This computer is up to date.', upd_check: 'Check', upd_checking: 'Checking…', upd_go: 'Update ({n})', upd_checked: 'Last check: {d}',
  upd_managed: 'Updates for this computer are managed by your administrator: you can see them here, but not start them.',
  upd_reboot: 'A restart is needed to finish the updates.', upd_security: 'Security',
  confirm_upd: 'Update {n} package(s)?', upd_body: 'The agent installs them with a restore point and rolls back if something goes wrong. The computer may ask for a restart afterwards.',
  upd_running: 'Updating the system', ok_upd: 'Updates installed', fail_upd: 'The update failed: {m}', upd_sent: 'Update requested ({n} package(s)).',
  check_started: 'Looking for updates…', check_done: 'Check finished: {n} update(s) available.', upd_unavailable: 'Updates are not available on this computer ({e}).',
  theme_system: 'Theme: follow the system', theme_light: 'Theme: light', theme_dark: 'Theme: dark', lang_title: 'Language: English (click for French)',
};

// ---- language and theme: the system's, until the user chooses (remembered on this machine)
const stored = (k) => { try { return localStorage.getItem(k); } catch { return null; } };
const store = (k, v) => { try { localStorage.setItem(k, v); } catch { /* private mode: the choice lasts for this session */ } };
let lang = ['fr', 'en'].includes(stored('cp.lang')) ? stored('cp.lang') : (navigator.language || 'fr').toLowerCase().startsWith('fr') ? 'fr' : 'en';
let W = lang === 'fr' ? FR : EN;
const t = (k, v = {}) => W[k].replace(/\{(\w+)\}/g, (_, x) => String(v[x] ?? ''));
document.documentElement.lang = lang;

let themeChoice = ['system', 'light', 'dark'].includes(stored('cp.theme')) ? stored('cp.theme') : 'system';
const dark = window.matchMedia('(prefers-color-scheme: dark)');
const applyTheme = () => { document.documentElement.dataset.theme = themeChoice === 'system' ? (dark.matches ? 'dark' : 'light') : themeChoice; };
applyTheme();
dark.addEventListener('change', applyTheme);

// ---- tiny DOM helpers
const $ = (id) => document.getElementById(id);
function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === 'class') e.className = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? '' : v);
  }
  for (const k of kids.flat()) if (k !== null && k !== undefined && k !== false) e.append(k.nodeType ? k : document.createTextNode(String(k)));
  return e;
}
function icon(name, cls = 'ico') {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 256 256');
  svg.setAttribute('class', cls);
  svg.setAttribute('aria-hidden', 'true');
  for (const d of ICONS[name] ?? ICONS.cube) {
    const p = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    p.setAttribute('d', d);
    svg.append(p);
  }
  return svg;
}
document.querySelectorAll('[data-icon]').forEach((n) => n.replaceWith(icon(n.dataset.icon)));
/** The tile of an app: its logo or uploaded image when the catalog has one (an <img>: an SVG cannot run scripts there), else a glyph. */
function appIcon(a) {
  const src = a.icon_url || (typeof a.icon === 'string' && /^data:image\/(png|jpeg|webp|svg\+xml);base64,/.test(a.icon) ? a.icon : null);
  if (src) return el('span', { class: 'card__icon card__icon--img' }, el('img', { src, alt: '', width: 32, height: 32 }));
  return el('span', { class: 'card__icon' }, icon(a.icon || 'cube', 'ico ico--lg'));
}

const THEME_ICON = { system: 'auto', light: 'sun', dark: 'moon' };

// ---- backend
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

// ---- state
const S = { host: '', apps: [], cat: '', q: '', loaded: false, error: '', seen: new Map(), notes: [], busy: false, view: 'apps', upd: null, updOff: new Set(), updSeen: new Map(), updLoaded: false, checking: null };

function presence(a) {
  const l = a.local;
  if (l && l.total > 0) return l.present === l.total ? 'present' : l.present > 0 ? 'partial' : 'absent';
  return a.installed ? 'present' : 'absent';
}

function note(tone, text) {
  S.notes.unshift({ tone, text, at: new Date() });
  S.notes.length = Math.min(S.notes.length, 50);
  renderLog();
}

/** Report each finished operation once; operations from before the window opened are only remembered. */
function trackRecent(recent) {
  for (const r of recent ?? []) {
    const prev = S.seen.get(r.task_id);
    S.seen.set(r.task_id, r.status);
    if (!S.loaded || prev === r.status) continue;
    const n = r.app_name ?? '?';
    if (r.status === 'succeeded') note('ok', t(r.install === false ? 'ok_uninstall' : 'ok_install', { n }));
    else if (r.status === 'failed' || r.status === 'blocked' || r.status === 'rejected') note('err', t('failed', { n, m: r.message }));
    else if (r.status === 'cancelled' || r.status === 'expired') note('warn', t('cancelled', { n }));
  }
}

/** Report each finished system update once (not the ones from before the window opened). */
function trackUpdates(recent) {
  for (const r of recent ?? []) {
    const prev = S.updSeen.get(r.task_id);
    S.updSeen.set(r.task_id, r.status);
    if (!S.updLoaded || prev === r.status) continue;
    if (r.status === 'succeeded') note('ok', t('ok_upd'));
    else if (['failed', 'rolled_back', 'blocked', 'rejected'].includes(r.status)) note('err', t('fail_upd', { m: r.message }));
  }
}

async function refreshUpdates() {
  try {
    const u = await invoke('updates');
    S.upd = u;
    trackUpdates(u.recent);
    S.updLoaded = true;
    if (S.checking && (u.checked_at !== S.checking.from || Date.now() > S.checking.until)) {
      if (u.checked_at !== S.checking.from) note('info', t('check_done', { n: (u.pending ?? []).length }));
      S.checking = null;
    }
  } catch (e) {
    S.upd = null;
    S.updError = String(e);
  }
}

async function refresh() {
  try {
    const v = await invoke('list');
    S.error = '';
    S.host = v.host?.hostname ?? '';
    S.apps = v.apps ?? [];
    trackRecent(v.recent);
    if (!S.loaded) {
      const here = S.apps.filter((a) => presence(a) === 'present').map((a) => a.name);
      note('info', here.length ? t('opened', { l: here.join(', ') }) : t('none_present'));
    }
    S.loaded = true;
  } catch (e) {
    S.error = String(e);
  }
  await refreshUpdates();
  render();
  schedule();
}

let timer;
function schedule() {
  clearTimeout(timer);
  // Quick while something runs, so progress and the outcome show up promptly.
  timer = setTimeout(refresh, S.apps.some((a) => a.pending) || S.upd?.open_task || S.checking ? 2500 : 15000);
}

// ---- actions
function ask(a, action) {
  const d = $('confirm');
  const body = $('confirm-body');
  body.replaceChildren();
  $('confirm-title').textContent = t(`confirm_${action}`, { n: a.name });
  const removing = action === 'uninstall';
  if (a.kind === 'command' && a.command) {
    body.append(el('p', {}, t('command')), el('pre', {}, removing ? a.command.uninstall : a.command.install));
    if (!removing) body.append(el('p', { class: 'muted' }, t('check', { p: a.command.check })));
  } else {
    body.append(el('p', {}, t('packages')), el('pre', {}, (a.packages ?? []).join(' ')));
    if (action === 'complete' && a.local?.missing?.length) body.append(el('p', { class: 'muted' }, t('missing', { m: a.local.missing.join(', ') })));
  }
  const yes = $('confirm-yes');
  yes.textContent = t(removing ? 'uninstall' : action === 'complete' ? 'complete' : action === 'update' ? 'update' : 'install');
  yes.className = `btn ${removing ? 'btn--danger' : 'btn--primary'}`;
  $('confirm-no').textContent = t('cancel');
  d.onclose = () => { if (d.returnValue === 'ok') void send(a, removing ? 'uninstall' : 'install'); };
  d.returnValue = '';
  d.showModal();
  $('confirm-no').focus();
}

async function send(a, op) {
  S.busy = true;
  render();
  try {
    await invoke(op, { appId: a.id });
    note('info', t('sent', { n: a.name }));
  } catch (e) {
    note('err', t('failed', { n: a.name, m: String(e) }));
  }
  S.busy = false;
  await refresh();
}

// ---- rendering
function card(a, locked) {
  const p = presence(a);
  const pending = a.pending;
  const badge = pending
    ? el('span', { class: 'badge badge--info' }, icon('refresh'), t('working'))
    : p === 'present' ? el('span', { class: 'badge badge--ok' }, icon('check'), t('present'))
    : p === 'partial' ? el('span', { class: 'badge badge--warn' }, icon('warn'), t('partial', { p: a.local.present, t: a.local.total }))
    : el('span', { class: 'badge badge--neutral' }, t('available'));
  const actions = el('div', { class: 'card__actions' });
  if (!pending) {
    const dis = locked || S.busy;
    if (p !== 'present') actions.append(el('button', { class: 'btn btn--primary btn--sm', type: 'button', disabled: dis, onclick: () => ask(a, p === 'partial' ? 'complete' : 'install') }, icon('download'), t(p === 'partial' ? 'complete' : 'install')));
    // software installed by a command is not in the package manager's update list: the agent says when its source has a newer version
    if (p === 'present' && a.local?.update) actions.append(el('button', { class: 'btn btn--primary btn--sm', type: 'button', disabled: dis, onclick: () => ask(a, 'update') }, icon('refresh'), t('update')));
    if (p !== 'absent') actions.append(el('button', { class: 'btn btn--outline btn--sm', type: 'button', disabled: dis, onclick: () => ask(a, 'uninstall') }, icon('trash'), t('uninstall')));
  }
  const what = a.kind === 'command'
    ? el('span', { class: 'badge badge--cmd', title: a.command?.install ?? '' }, t('command_badge'))
    : el('span', { class: 'card__what mono' }, (a.packages ?? []).join(' · '));
  return el('article', { class: `card${pending ? ' card--busy' : ''}`, 'data-app': a.name },
    el('div', { class: 'card__head' },
      appIcon(a),
      el('div', {}, el('div', { class: 'card__name' }, a.name), el('div', { class: 'card__sub' }, [a.category, a.version || a.local?.installed_version].filter(Boolean).join(' · ')))),
    a.description ? el('p', { class: 'card__desc' }, a.description) : el('p', { class: 'card__desc' }),
    what,
    el('div', { class: 'card__foot' }, badge, !pending && p === 'present' && a.local?.update ? el('span', { class: 'badge badge--warn' }, icon('warn'), t('update_avail', { v: a.local.latest_version })) : null, actions));
}

function render() {
  $('title').textContent = t('title');
  $('host').textContent = S.host ? t('host', { h: S.host }) : '';
  $('q').placeholder = t('search');
  $('refresh-label').textContent = t('refresh');
  $('status-title').textContent = t('status');
  $('clear-label').textContent = t('clear');
  $('lang-label').textContent = lang.toUpperCase();
  $('lang').title = t('lang_title');
  $('theme').title = t(`theme_${themeChoice}`);
  $('theme-label').textContent = t(`theme_${themeChoice}`);
  $('theme-icon').replaceChildren(icon(THEME_ICON[themeChoice]));

  const notice = $('notice');
  notice.hidden = !S.error;
  if (S.error) notice.replaceChildren(icon('x', 'ico ico--md'), el('span', {}, S.error.includes('unreachable') ? t('unreachable', { e: S.error }) : S.error));

  // categories
  const cats = [...new Set(S.apps.map((a) => a.category).filter(Boolean))].sort((x, y) => x.localeCompare(y));
  if (S.cat && !cats.includes(S.cat)) S.cat = '';
  const catBtn = (c, label, n) => el('button', { class: 'cat', type: 'button', 'aria-current': S.cat === c ? 'true' : 'false', onclick: () => { S.cat = c; render(); } }, el('span', {}, label), el('span', { class: 'cat__count' }, n));
  $('cats').replaceChildren(catBtn('', t('all'), S.apps.length), ...cats.map((c) => catBtn(c, c, S.apps.filter((a) => a.category === c).length)));

  // apps
  const q = S.q.trim().toLowerCase();
  const shown = S.apps.filter((a) => (!S.cat || a.category === S.cat) && (!q || `${a.name} ${a.description} ${(a.packages ?? []).join(' ')}`.toLowerCase().includes(q)));
  const locked = S.apps.some((a) => a.pending); // one operation at a time per machine
  $('grid').replaceChildren(...shown.map((a) => card(a, locked)));
  const empty = $('empty');
  empty.hidden = !S.loaded || shown.length > 0;
  empty.textContent = S.apps.length ? t('no_match') : t('empty');

  // progress of the running operation: an app install or a system update
  const run = S.apps.find((a) => a.pending);
  const up = S.upd?.open_task;
  $('progress').hidden = !run && !up;
  $('idle').hidden = !!run || !!up;
  $('idle').textContent = t('idle');
  const stage = (st) => ({ queued: 20, sent: 45, running: 75, soaking: 90 })[st] ?? 60;
  if (run) {
    const st = run.pending.status;
    $('progress-text').textContent = t(run.pending.install === false ? 'removing' : 'installing', { n: run.name });
    $('progress-stage').textContent = W[`stage_${st}`] ?? st;
    $('progress-bar').style.width = `${stage(st)}%`;
  } else if (up) {
    $('progress-text').textContent = t('upd_running');
    $('progress-stage').textContent = W[`stage_${up.status}`] ?? up.status;
    $('progress-bar').style.width = `${stage(up.status)}%`;
  }
  renderViews();
}

function renderViews() {
  const pending = S.upd?.pending ?? [];
  $('tab-apps').textContent = t('tab_apps');
  $('tab-updates-label').textContent = t('tab_updates');
  $('tab-apps').setAttribute('aria-selected', String(S.view === 'apps'));
  $('tab-updates').setAttribute('aria-selected', String(S.view === 'updates'));
  const n = $('tab-updates-n');
  n.hidden = pending.length === 0;
  n.textContent = String(pending.length);
  document.querySelector('.layout').hidden = S.view !== 'apps';
  document.querySelector('.search').hidden = S.view !== 'apps';
  $('updates').hidden = S.view !== 'updates';
  if (S.view === 'updates') renderUpdates();
}

function selectedUpdates() {
  return (S.upd?.pending ?? []).map((p) => p.name).filter((nme) => !S.updOff.has(nme));
}

function renderUpdates() {
  const u = S.upd;
  const pending = u?.pending ?? [];
  const sec = pending.filter((p) => p.is_security).length;
  const busy = !!u?.open_task;
  $('upd-title').textContent = t('upd_title');
  $('upd-sub').textContent = !u ? t('upd_unavailable', { e: S.updError ?? '' }) : [pending.length ? t('upd_sub', { n: pending.length, sec: sec ? t('upd_sec', { s: sec }) : '' }) : t('upd_none'), u.checked_at ? t('upd_checked', { d: new Date(u.checked_at).toLocaleString(document.documentElement.lang) }) : ''].filter(Boolean).join(' · ');
  $('upd-check-label').textContent = S.checking ? t('upd_checking') : t('upd_check');
  $('upd-check').disabled = !!S.checking || !u;
  const chosen = selectedUpdates();
  $('upd-go-label').textContent = t('upd_go', { n: chosen.length });
  $('upd-go').hidden = !u?.allowed;
  $('upd-go').disabled = !u?.allowed || busy || chosen.length === 0 || S.busy;
  $('upd-managed').hidden = !u || u.allowed;
  $('upd-managed').textContent = t('upd_managed');
  $('upd-reboot').hidden = !u?.reboot_required;
  $('upd-reboot').textContent = t('upd_reboot');
  $('upd-empty').hidden = !u || pending.length > 0;
  $('upd-empty').textContent = t('upd_none');
  $('upd-list').replaceChildren(...pending.map((p) => el('li', { 'data-update': p.name },
    el('input', { type: 'checkbox', checked: !S.updOff.has(p.name) || undefined, disabled: !u.allowed || busy, 'aria-label': p.name, onchange: (e) => { if (e.target.checked) S.updOff.delete(p.name); else S.updOff.add(p.name); renderUpdates(); } }),
    el('span', { class: 'name' }, p.name),
    el('span', { class: 'ver' }, `${p.version} → ${p.candidate ?? '?'}`),
    p.is_security ? el('span', { class: 'badge badge--warn' }, icon('warn'), t('upd_security')) : null)));
}

function renderLog() {
  const glyph = { ok: 'check', err: 'x', warn: 'warn', info: 'info' };
  $('log').replaceChildren(...S.notes.map((n) => el('li', {},
    el('time', {}, n.at.toLocaleTimeString(document.documentElement.lang, { hour: '2-digit', minute: '2-digit', second: '2-digit' })),
    el('span', { class: n.tone }, icon(glyph[n.tone])),
    el('span', {}, n.text))));
}

// ---- wiring
$('tab-apps').addEventListener('click', () => { S.view = 'apps'; render(); });
$('tab-updates').addEventListener('click', () => { S.view = 'updates'; render(); });
$('upd-check').addEventListener('click', async () => {
  S.checking = { from: S.upd?.checked_at ?? null, until: Date.now() + 120000 };
  note('info', t('check_started'));
  render();
  try { await invoke('check'); } catch (e) { S.checking = null; note('err', String(e)); }
  render();
  schedule();
});
$('upd-go').addEventListener('click', () => {
  const names = selectedUpdates();
  const d = $('confirm');
  $('confirm-body').replaceChildren(el('p', {}, t('upd_body')), el('pre', {}, names.join(' ')));
  $('confirm-title').textContent = t('confirm_upd', { n: names.length });
  const yes = $('confirm-yes');
  yes.textContent = t('upd_go', { n: names.length });
  yes.className = 'btn btn--primary';
  $('confirm-no').textContent = t('cancel');
  d.onclose = async () => {
    if (d.returnValue !== 'ok') return;
    S.busy = true;
    render();
    try {
      await invoke('update', { packages: names });
      note('info', t('upd_sent', { n: names.length }));
    } catch (e) {
      note('err', t('fail_upd', { m: String(e) }));
    }
    S.busy = false;
    await refresh();
  };
  d.returnValue = '';
  d.showModal();
  $('confirm-no').focus();
});
$('lang').addEventListener('click', () => {
  lang = lang === 'fr' ? 'en' : 'fr';
  W = lang === 'fr' ? FR : EN;
  store('cp.lang', lang);
  document.documentElement.lang = lang;
  render();
  renderLog();
});
$('theme').addEventListener('click', () => {
  themeChoice = ({ system: 'light', light: 'dark', dark: 'system' })[themeChoice];
  store('cp.theme', themeChoice);
  applyTheme();
  render();
});
$('q').addEventListener('input', (e) => { S.q = e.target.value; render(); });
$('refresh').addEventListener('click', () => void refresh());
$('clear').addEventListener('click', () => { S.notes = []; renderLog(); });
document.addEventListener('keydown', (e) => {
  if (e.key === '/' && document.activeElement !== $('q') && !$('confirm').open) { e.preventDefault(); $('q').focus(); }
  if ((e.key === 'F5' || (e.ctrlKey && e.key === 'r')) && !$('confirm').open) { e.preventDefault(); void refresh(); }
});
window.addEventListener('focus', () => void refresh());

render();
renderLog();
void refresh();

// The version of this window, in the header.
invoke('version').then((v) => { $('version').textContent = `v${v}`; }).catch(() => {});
