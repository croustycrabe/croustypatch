// CroustyPatch desktop app center. Talks to the agent's local socket through three Tauri commands
// (list, install, uninstall); the agent identifies the user from the socket, never from this window.
import { ICONS } from './icons.js';

const FR = {
  title: 'Portail applicatif', host: 'Machine {h} · demandes au nom de votre compte', all: 'Toutes', search: 'Rechercher une application',
  refresh: 'Actualiser', present: 'Présent', partial: 'Partiel {p}/{t}', available: 'Disponible', working: 'En cours…',
  install: 'Installer', uninstall: 'Désinstaller', complete: 'Compléter', cancel: 'Annuler',
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
};
const EN = {
  title: 'App center', host: 'Machine {h} · requests made as your account', all: 'All', search: 'Search an application',
  refresh: 'Refresh', present: 'Present', partial: 'Partial {p}/{t}', available: 'Available', working: 'Working…',
  install: 'Install', uninstall: 'Uninstall', complete: 'Complete', cancel: 'Cancel',
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
};
const W = (navigator.language || 'fr').toLowerCase().startsWith('fr') ? FR : EN;
const t = (k, v = {}) => W[k].replace(/\{(\w+)\}/g, (_, x) => String(v[x] ?? ''));
document.documentElement.lang = W === FR ? 'fr' : 'en';

// ---- theme follows the system
const dark = window.matchMedia('(prefers-color-scheme: dark)');
const applyTheme = () => { document.documentElement.dataset.theme = dark.matches ? 'dark' : 'light'; };
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

// ---- backend
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

// ---- state
const S = { host: '', apps: [], cat: '', q: '', loaded: false, error: '', seen: new Map(), notes: [], busy: false };

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
  render();
  schedule();
}

let timer;
function schedule() {
  clearTimeout(timer);
  // Quick while something runs, so progress and the outcome show up promptly.
  timer = setTimeout(refresh, S.apps.some((a) => a.pending) ? 2500 : 15000);
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
  yes.textContent = t(removing ? 'uninstall' : action === 'complete' ? 'complete' : 'install');
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
    if (p !== 'absent') actions.append(el('button', { class: 'btn btn--outline btn--sm', type: 'button', disabled: dis, onclick: () => ask(a, 'uninstall') }, icon('trash'), t('uninstall')));
  }
  const what = a.kind === 'command'
    ? el('span', { class: 'badge badge--cmd', title: a.command?.install ?? '' }, t('command_badge'))
    : el('span', { class: 'card__what mono' }, (a.packages ?? []).join(' · '));
  return el('article', { class: `card${pending ? ' card--busy' : ''}`, 'data-app': a.name },
    el('div', { class: 'card__head' },
      el('span', { class: 'card__icon' }, icon(a.icon || 'cube', 'ico ico--lg')),
      el('div', {}, el('div', { class: 'card__name' }, a.name), el('div', { class: 'card__sub' }, [a.category, a.version].filter(Boolean).join(' · ')))),
    a.description ? el('p', { class: 'card__desc' }, a.description) : el('p', { class: 'card__desc' }),
    what,
    el('div', { class: 'card__foot' }, badge, actions));
}

function render() {
  $('title').textContent = t('title');
  $('host').textContent = S.host ? t('host', { h: S.host }) : '';
  $('q').placeholder = t('search');
  $('refresh-label').textContent = t('refresh');
  $('status-title').textContent = t('status');
  $('clear-label').textContent = t('clear');

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

  // progress of the running operation
  const run = S.apps.find((a) => a.pending);
  $('progress').hidden = !run;
  $('idle').hidden = !!run;
  $('idle').textContent = t('idle');
  if (run) {
    const st = run.pending.status;
    $('progress-text').textContent = t(run.pending.install === false ? 'removing' : 'installing', { n: run.name });
    $('progress-stage').textContent = W[`stage_${st}`] ?? st;
    $('progress-bar').style.width = `${({ queued: 20, sent: 45, running: 75, soaking: 90 })[st] ?? 60}%`;
  }
}

function renderLog() {
  const glyph = { ok: 'check', err: 'x', warn: 'warn', info: 'info' };
  $('log').replaceChildren(...S.notes.map((n) => el('li', {},
    el('time', {}, n.at.toLocaleTimeString(document.documentElement.lang, { hour: '2-digit', minute: '2-digit', second: '2-digit' })),
    el('span', { class: n.tone }, icon(glyph[n.tone])),
    el('span', {}, n.text))));
}

// ---- wiring
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
