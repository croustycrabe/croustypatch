// Browser checks for the desktop app center UI (portal-gui/ui) without webkit: the real HTML/JS is served over HTTP
// and Tauri's `invoke` is replaced by a mock that answers like the agent's socket (same payloads).
//   cd frontend && node e2e/portal-gui.mjs        (screenshots land in e2e/.run/portal-gui-*.png)
import { chromium } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const UI = path.join(HERE, '../../portal-gui/ui');
const OUT = path.join(HERE, '.run');
fs.mkdirSync(OUT, { recursive: true });
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
const CSP = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'";
const server = http.createServer((req, res) => {
  const f = path.join(UI, decodeURIComponent(new URL(req.url, 'http://x').pathname === '/' ? '/index.html' : new URL(req.url, 'http://x').pathname));
  if (!f.startsWith(UI) || !fs.existsSync(f)) { res.writeHead(404); return res.end(); }
  // The same policy as tauri.conf.json, plus a mock script that tests inject through addInitScript (allowed: CDP).
  res.writeHead(200, { 'content-type': TYPES[path.extname(f)] ?? 'application/octet-stream', 'content-security-policy': CSP });
  fs.createReadStream(f).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const BASE = `http://127.0.0.1:${server.address().port}/`;

/** Mock of the agent socket behind the three Tauri commands. Runs inside the page. */
function mock() {
  const apps = [
    { id: 'a1', name: 'Git', version: '', category: 'Développement', description: 'Gestion de versions distribuée.', icon: 'code', kind: 'packages', packages: ['git'], command: null, installed: true, pending: null, local: { present: 1, total: 1, missing: [] } },
    { id: 'a2', name: 'Outils de compilation', version: '', category: 'Développement', description: 'Compilateur C/C++, make et outils de construction.', icon: 'wrench', kind: 'packages', packages: ['base-devel', 'cmake'], command: null, installed: false, pending: null, local: { present: 1, total: 2, missing: ['cmake'] } },
    { id: 'a3', name: 'htop', version: '3.4', category: 'Système', description: 'Moniteur de processus interactif.', icon: 'chart-bar', kind: 'packages', packages: ['htop'], command: null, installed: false, pending: null, local: { present: 0, total: 1, missing: ['htop'] } },
    { id: 'a4', name: 'Outil interne', version: '2.1', category: 'Métier', description: 'Client de l’intranet, installé par le script maison.', icon: 'cube', kind: 'command', packages: [], command: { install: 'curl -fsSL https://intranet/outil.tar.gz | tar -xz -C /opt\nln -sf /opt/outil/bin/outil /usr/local/bin/outil', uninstall: 'rm -rf /opt/outil /usr/local/bin/outil', check: '/usr/local/bin/outil' }, installed: false, pending: null, local: { present: 0, total: 1, missing: ['/usr/local/bin/outil'] } },
    { id: 'a5', name: 'VLC', version: '', category: 'Multimédia', description: 'Lecteur audio et vidéo.', icon: 'music-notes', kind: 'packages', packages: ['vlc'], command: null, installed: false, pending: null, local: { present: 0, total: 1, missing: ['vlc'] } },
  ];
  const recent = [{ task_id: 'old', status: 'succeeded', message: '1 package(s) installed', app_id: 'a1', app_name: 'Git', install: true }];
  let ticks = 0;
  window.__calls = [];
  const step = () => {
    const a = apps.find((x) => x.pending);
    if (!a) return;
    ticks += 1;
    if (ticks === 1) a.pending.status = 'sent';
    else if (ticks === 2) a.pending.status = 'running';
    else {
      const install = a.pending.install;
      recent.unshift({ task_id: a.pending.task_id, status: 'succeeded', message: 'done', app_id: a.id, app_name: a.name, install });
      a.pending = null;
      a.local = { present: install ? a.local.total : 0, total: a.local.total, missing: [] };
      a.installed = install;
    }
  };
  window.__TAURI__ = { core: { invoke: async (cmd, args) => {
    window.__calls.push([cmd, args]);
    if (window.__unreachable) throw 'agent unreachable at /run/croustypatch/portal.sock (No such file or directory)';
    if (cmd === 'list') { step(); return JSON.parse(JSON.stringify({ host: { id: 'h1', hostname: 'laptop-alice' }, apps, recent })); }
    const a = apps.find((x) => x.id === args.appId);
    if (!a) throw 'this application is not offered on this host';
    if (apps.some((x) => x.pending)) throw 'an install or uninstall is already running on this host';
    ticks = 0;
    a.pending = { task_id: `t${Date.now()}`, status: 'queued', install: cmd === 'install', message: '' };
    return { status: 'queued', task_id: a.pending.task_id };
  } } };
}

const res = [];
const check = (n, ok, x = '') => res.push(`${ok ? 'PASS' : 'FAIL'}  ${n}${x ? '  — ' + x : ''}`);
const browser = await chromium.launch();
const errors = [];
const open = async ({ locale = 'fr-FR', scheme = 'light', vp = { width: 1120, height: 740 } } = {}) => {
  const ctx = await browser.newContext({ locale, colorScheme: scheme, viewport: vp });
  const p = await ctx.newPage();
  p.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  p.on('pageerror', (e) => errors.push(String(e)));
  await p.addInitScript(mock);
  await p.goto(BASE);
  await p.waitForSelector('.card');
  return p;
};

const p = await open();
check('the catalog loads from the agent socket', (await p.locator('.card').count()) === 5);
check('the host is named in the header', (await p.locator('#host').textContent()).includes('laptop-alice'));
check('presence comes from the machine: present, partial, available', (await p.locator('[data-app="Git"] .badge--ok').count()) === 1 && (await p.locator('[data-app="Outils de compilation"] .badge--warn').textContent()).includes('1/2') && (await p.locator('[data-app="htop"] .badge--neutral').count()) === 1);
check('the opening note lists what is already installed', (await p.locator('#log li').first().textContent()).includes('Git'));
check('the note does not repeat operations from before the window opened', (await p.locator('#log li').count()) === 1);
await p.screenshot({ path: `${OUT}/portal-gui-light-fr.png` });

await p.click('.cat:has-text("Système")');
check('categories filter the catalog', (await p.locator('.card').count()) === 1);
await p.click('.cat:has-text("Toutes")');
await p.fill('#q', 'vlc');
check('search filters by name', (await p.locator('.card').count()) === 1);
await p.fill('#q', '');

// a command app shows exactly what will run, and the path checked for this account
await p.click('[data-app="Outil interne"] button:has-text("Installer")');
await p.waitForSelector('#confirm[open]');
const body = await p.locator('#confirm-body').textContent();
check('the confirmation shows the command that will run', body.includes('tar -xz -C /opt') && body.includes('/usr/local/bin/outil'));
await p.screenshot({ path: `${OUT}/portal-gui-confirm.png` });
await p.click('#confirm-no');
check('cancelling sends nothing', !(await p.evaluate(() => window.__calls.some(([c]) => c === 'install'))));

// install: request, progress, outcome
await p.click('[data-app="htop"] button:has-text("Installer")');
await p.waitForSelector('#confirm[open]');
check('a package app lists its packages before installing', (await p.locator('#confirm-body pre').textContent()).trim() === 'htop');
await p.click('#confirm-yes');
await p.waitForSelector('#progress:not([hidden])');
check('the request is sent with the app id only', await p.evaluate(() => JSON.stringify(window.__calls.find(([c]) => c === 'install')) === JSON.stringify(['install', { appId: 'a3' }])));
check('a progress bar follows the operation', (await p.locator('#progress-text').textContent()).includes('htop'));
check('other actions wait while an operation runs', await p.locator('[data-app="VLC"] button:has-text("Installer")').isDisabled());
await p.screenshot({ path: `${OUT}/portal-gui-progress.png` });
await p.waitForSelector('[data-app="htop"] .badge--ok', { timeout: 15000 }).then(() => check('the outcome shows as present', true), () => check('the outcome shows as present', false));
check('the outcome is notified once', (await p.locator('#log li:has-text("htop installé")').count()) === 1);
check('the progress bar goes away when done', await p.locator('#progress').isHidden());

// partial app: complete
await p.click('[data-app="Outils de compilation"] button:has-text("Compléter")');
await p.waitForSelector('#confirm[open]');
check('completing a partial app names what is missing', (await p.locator('#confirm-body').textContent()).includes('cmake'));
await p.keyboard.press('Escape');

// agent down
await p.evaluate(() => { window.__unreachable = true; });
await p.click('#refresh');
await p.waitForSelector('#notice:not([hidden])');
check('an unreachable agent is explained', (await p.locator('#notice').textContent()).includes('ne répond pas'));
await p.evaluate(() => { window.__unreachable = false; });
await p.click('#refresh');
await p.waitForSelector('#notice[hidden]', { state: 'attached' });
check('and the notice goes away once it answers again', true);

// language and theme switches: a choice is remembered; otherwise the system's applies
{
  const sw = await open({ locale: 'fr-FR', scheme: 'light' });
  const theme = () => sw.evaluate(() => document.documentElement.dataset.theme);
  check('the language switch shows the current language', (await sw.locator('#lang-label').textContent()) === 'FR');
  await sw.click('#lang');
  check('switching language translates the whole window', (await sw.locator('#title').textContent()) === 'App center' && (await sw.locator('#lang-label').textContent()) === 'EN' && (await sw.locator('[data-app="htop"] button').first().textContent()).includes('Install'));
  await sw.reload(); await sw.waitForSelector('.card');
  check('the language choice is remembered', (await sw.locator('#title').textContent()) === 'App center');
  check('the theme starts with the system (light)', (await theme()) === 'light');
  await sw.click('#theme'); check('first click: light, whatever the system says', (await theme()) === 'light');
  await sw.click('#theme'); check('second click: dark', (await theme()) === 'dark');
  await sw.reload(); await sw.waitForSelector('.card');
  check('the theme choice is remembered', (await theme()) === 'dark');
  await sw.click('#theme'); check('third click: back to following the system', (await theme()) === 'light');
  await sw.screenshot({ path: `${OUT}/portal-gui-switches.png` });
  await sw.context().close();
}

const d = await open({ locale: 'en-US', scheme: 'dark' });
check('English follows the system language', (await d.locator('#title').textContent()) === 'App center' && (await d.locator('[data-app="htop"] button').first().textContent()).includes('Install'));
check('dark theme follows the system', (await d.evaluate(() => document.documentElement.dataset.theme)) === 'dark' && (await d.evaluate(() => getComputedStyle(document.body).backgroundColor)) !== 'rgb(245, 242, 234)');
await d.screenshot({ path: `${OUT}/portal-gui-dark-en.png` });
const n = await open({ vp: { width: 760, height: 640 } });
check('a narrow window keeps every app reachable', (await n.locator('.card').count()) === 5 && (await n.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)));
await n.screenshot({ path: `${OUT}/portal-gui-narrow.png` });

check('no script or CSP error in the console', errors.length === 0, errors.slice(0, 3).join(' | '));
console.log(res.join('\n'));
const failed = res.filter((r) => r.startsWith('FAIL')).length;
console.log(`\n${res.length - failed} passed, ${failed} failed`);
process.exitCode = failed ? 1 : 0;
await browser.close();
server.close();
