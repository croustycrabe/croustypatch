import { chromium } from 'playwright';
import { spawn, execFileSync } from 'node:child_process';
import http from 'node:http';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
const HERE = path.dirname(fileURLToPath(import.meta.url));
const U = path.join(HERE, '.run');
const AGENT = process.env.AGENT_BIN ?? path.join(HERE, '../../target/debug/croustypatch-agent');
const BASE = process.env.E2E_BASE ?? 'https://localhost:8443';
const browser = await chromium.launch();
const res = [];
const check = (n, ok, x = '') => res.push(`${ok ? 'PASS' : 'FAIL'}  ${n}${x ? '  — ' + x : ''}`);
const open = async (vp = { width: 1440, height: 900 }) => (await browser.newContext({ ignoreHTTPSErrors: true, viewport: vp, locale: 'fr-FR' })).newPage();
const login = async (p, u, pw) => { await p.goto(`${BASE}/login/`); await p.fill('input[autocomplete=username]', u); await p.fill('input[autocomplete=current-password]', pw); await p.click('button[type=submit]'); };

let p = await open();
await login(p, 'admin', 'wrong'); await p.waitForSelector('.alert--error'); check('bad credentials show an error on the login page', p.url().endsWith('/login/'));
await login(p, 'admin', 'e2e-admin-password'); await p.waitForSelector('.sidebar'); await p.waitForSelector('.stat');
check('dashboard shows fleet stat cards', await p.locator('.stat').count() >= 5);
check('every indicator is a tile of its own, plus the new panels', (await p.locator('[data-widget^=stat_]').count()) >= 6 && (await p.locator('[data-widget=updates], [data-widget=patching], [data-widget=resources]').count()) === 3);
{
  await p.click('button:has-text("Personnaliser")');
  check('customising offers the tiles and panels that are hidden', (await p.locator('button:has-text("Redémarrages requis")').count()) === 1);
  await p.click('button:has-text("Afficher · Redémarrages requis")');
  check('a hidden tile is shown with one click', (await p.locator('[data-widget=stat_reboot]').count()) === 1);
  await p.waitForFunction(() => { const r = document.querySelector('[data-widget=stat_reboot]')?.getBoundingClientRect(); return !!r && r.top >= 0 && r.bottom <= innerHeight; }, null, { timeout: 5000 }).then(() => check('and brought into view', true), () => check('and brought into view', false));
  await p.click('[data-widget=stat_reboot] button[aria-label="Masquer"]');
  check('and hidden again', (await p.locator('[data-widget=stat_reboot]').count()) === 0);
  await p.click('[data-widget=updates] button[aria-label="Masquer"]'); await p.click('button:has-text("Rétablir la disposition par défaut")');
  await p.waitForSelector('[data-widget=updates]');
  check('"Restore the default layout" brings everything back', (await p.locator('[data-widget=updates]').count()) === 1);
  await p.click('button:has-text("Terminer")');
}
check('"Add agent" is offered in the sidebar and on the page', await p.locator('button:has-text("Ajouter un agent")').count() >= 2);

// ---- navigation and hosts table
await p.click('.sidebar a.navitem:has-text("hôtes")'); await p.waitForSelector('.tbl');
const rows = () => p.locator('.tbl tbody tr').count();
const total = await rows();
await p.click('.chips button:has-text("quarantaine")'); await p.waitForTimeout(200);
check('status chip filters the table', (await rows()) === 1 && total > 1, `${total} -> ${await rows()}`);
await p.click('.chips button:has-text("Tout")'); await p.fill('input[type=search]', 'web-01'); await p.waitForTimeout(200);
check('search filters by host name', (await rows()) === 1);
await p.fill('input[type=search]', ''); await p.waitForTimeout(150);
const firstName = async () => (await p.locator('.tbl tbody tr .cell-host a').first().textContent());
const a0 = await firstName(); await p.click('th button:has-text("Hôte")'); await p.waitForTimeout(150); const a1 = await firstName();
check('clicking a column header sorts', a0 !== a1, `${a0} -> ${a1}`);
await p.locator('.tbl tbody tr input[type=checkbox]').nth(0).check(); await p.locator('.tbl tbody tr input[type=checkbox]').nth(1).check();
check('selecting rows shows the bulk action bar', (await p.locator('.alert:has-text("sélectionné")').count()) === 1 && (await p.locator('.alert button:has-text("Corriger")').count()) === 1);
await p.click('.alert button:has-text("Tout désélectionner")');

// ---- host detail tabs (state kept in the URL)
await p.click('.tbl a >> text=web-01'); await p.waitForSelector('.tabs');
await p.click('[role=tab]:has-text("Paquets")'); await p.waitForTimeout(300);
check('tab is reflected in the URL', p.url().includes('tab=packages'));
await p.reload(); await p.waitForSelector('.tabs'); check('and survives a reload', (await p.locator('[role=tab][aria-selected=true]').textContent()).includes('Paquets'));

// ---- command palette
await p.goto(`${BASE}/`); await p.waitForSelector('.stat'); await p.keyboard.press('Control+k'); await p.waitForSelector('dialog.palette[open]');
await p.keyboard.type('paquets'); await p.keyboard.press('Enter'); await p.waitForURL(/\/packages\/?$/);
check('⌘K palette jumps to a page', true);
await p.keyboard.press('Control+k'); await p.keyboard.type('db-01'); await p.waitForSelector('[role=option]:has-text("db-01")'); await p.keyboard.press('Enter'); await p.waitForURL(/\/host\/?\?id=/);
check('⌘K palette jumps to a host', (await p.locator('h1').textContent()).includes('db-01'));

// ---- packages: search, drawer, update across hosts
await p.goto(`${BASE}/packages/`); await p.waitForSelector('.tbl'); await p.fill('input[type=search]', 'libssl'); await p.waitForTimeout(500);
check('package search narrows the list', (await rows()) === 1);
await p.click('.tbl tbody tr >> nth=0'); await p.waitForSelector('dialog[open] .tbl'); 
check('package drawer lists the hosts carrying it', (await p.locator('dialog[open] .tbl tbody tr').count()) >= 2);
await p.click('dialog[open] button:has-text("Mettre à jour sur")'); await p.waitForSelector('dialog[open] .alert--error');
check('refused updates (open task / quarantine) are explained inline, not silently dropped', true);
await p.keyboard.press('Escape');
await p.fill('input[type=search]', 'firefox'); await p.waitForTimeout(500); await p.click('.tbl tbody tr >> nth=0'); await p.waitForSelector('dialog[open] .tbl');
check('a single host reads "1 hôte", not "1 hôtes"', (await p.locator('dialog[open] button:has-text("Mettre à jour sur 1 hôte")').count()) === 1);
await p.click('dialog[open] button:has-text("Mettre à jour sur 1 hôte")'); await p.waitForSelector('.toast');
check('"update on N hosts" creates tasks and confirms with a toast', (await p.locator('.toast').first().textContent()).includes('tâche'));
await p.keyboard.press('Escape');

// ---- tasks: approve from the slide-over
await p.goto(`${BASE}/tasks/?filter=approval`); await p.waitForSelector('.tbl a');
await p.click('.tbl a >> nth=0'); await p.waitForSelector('dialog[open] .timeline'); 
check('task detail opens as a side panel with the decision reasoning', (await p.locator('dialog.drawer[open]').count()) === 1 && (await p.locator('dialog[open] li:has-text("stability")').count()) > 0);
await p.click('dialog[open] button:has-text("Approuver")');
await p.waitForFunction(() => /en file|envoyée|en cours|observation|réussie/i.test(document.querySelector('dialog[open] .status')?.textContent ?? ''), null, { timeout: 5000 }).catch(() => {});
check('approving moves the task on', (await p.locator('dialog[open] .status').first().textContent()).match(/en file|envoyée|en cours|observation|réussie/i) !== null, await p.locator('dialog[open] .status').first().textContent());
await p.keyboard.press('Escape');

// ---- policy editor drawer
await p.goto(`${BASE}/policies/`); await p.waitForSelector('.tbl'); await p.click('button:has-text("Modifier") >> nth=0'); await p.waitForSelector('dialog[open] .section');
await p.fill('dialog[open] input >> nth=0', 'default (edited)'); await p.click('dialog[open] .sticky-actions button[type=submit]'); await p.waitForSelector('.toast, tr:has-text("default (edited)")', { timeout: 6000 }).catch(() => {});
await p.waitForTimeout(500); check('policy saved from the slide-over', (await p.locator('tr:has-text("default (edited)")').count()) === 1);
// patch days: a clear "when" card, presets, a time zone and a live preview from the server
await p.click('button:has-text("Nouvelle politique")'); await p.waitForSelector('dialog[open] :text("Quand les mises à jour sont-elles appliquées")');
await p.click('dialog[open] button:has-text("Elle est appliquée automatiquement")');
await p.click('dialog[open] button:has-text("Seulement les jours de patch")');
await p.click('dialog[open] button:has-text("Premier lundi du mois")');
await p.waitForFunction(() => document.querySelectorAll('dialog[open] fieldset li').length >= 5, null, { timeout: 8000 }).catch(() => {});
{
  const lis = await p.locator('dialog[open] fieldset li').allTextContents();
  check('the patch-day preview lists the next five patch days, all on a Monday', lis.length === 5 && lis.every((x) => /lun/i.test(x)), JSON.stringify(lis.slice(0, 2)));
  check('the summary says it in words', (await p.locator('dialog[open] .alert').last().textContent()).toLowerCase().includes('premier lundi'));
  await p.fill('dialog[open] input[list=cp-zones]', 'Mars/Olympus'); await p.waitForSelector('dialog[open] .alert--error', { timeout: 8000 }).then(() => check('an unknown time zone is refused with a reason', true), () => check('an unknown time zone is refused with a reason', false));
}
await p.keyboard.press('Escape');

// ---- ADD AGENT: generate, enrol a real agent with the command, watch the wizard notice
await p.goto(`${BASE}/hosts/`); await p.waitForSelector('.tbl'); await p.click('.sidebar button:has-text("Ajouter un agent")'); await p.waitForSelector('dialog[open]');
check('the wizard offers the modules for the new host', await p.locator('dialog[open] fieldset:has-text("Modules") input[type=checkbox]').count() === 5);
await p.check('dialog[open] fieldset:has-text("Modules") label:has-text("Docker") input');
await p.click('dialog[open] button:has-text("Générer la commande")'); await p.waitForSelector('dialog[open] pre.cmd');
const cmd = await p.locator('dialog[open] pre.cmd').first().textContent();
check('the Linux command is a curl one-liner for this server', /^curl -fsSL https:\/\/localhost:8443\/api\/v1\/agent\/install\.sh \| sudo env CROUSTYPATCH_ENROLL_TOKEN=cpt_\S+ sh$/.test(cmd), cmd.slice(0, 90));
const token = cmd.match(/cpt_[A-Za-z0-9_-]+/)[0];
await p.click('dialog[open] [role=tab]:has-text("FreeBSD")'); check('the FreeBSD tab uses fetch(1)', (await p.locator('dialog[open] pre.cmd').first().textContent()).startsWith('fetch -qo - '));
await p.click('dialog[open] [role=tab]:has-text("Manuel")'); check('the manual tab shows download + checksum + enrol steps', (await p.locator('dialog[open] pre.cmd').first().textContent()).includes('sha256'));
check('missing platform binaries are reported', await p.locator('dialog[open] .tag.tone-danger').count() >= 1);
await p.click('dialog[open] [role=tab]:has-text("Linux")');
execFileSync('rm', ['-rf', `${U}/wiz-state`]);
execFileSync('sh', ['-c', `printf 'package_manager = "mock"\\nmock_file = "${U}/wiz.mock.json"\\nstate_dir = "${U}/wiz-state"\\nheartbeat_secs = 10\\ndry_run = true\\n' > ${U}/wiz.toml; echo '{"installed":[{"name":"bash","version":"5.2"}]}' > ${U}/wiz.mock.json`]);
execFileSync(AGENT, ['--config', `${U}/wiz.toml`, 'enroll', '--server', BASE, '--ca-file', `${U}/data/tls/server.crt`], { env: { ...process.env, CROUSTYPATCH_ENROLL_TOKEN: token } });
const agent = spawn(AGENT, ['--config', `${U}/wiz.toml`, 'run'], { stdio: 'ignore' });
await p.waitForSelector('dialog[open] :text("est connecté")', { timeout: 15000 }).then(() => check('the wizard detects the new agent and confirms the connection', true), () => check('the wizard detects the new agent and confirms the connection', false));
check('and offers to open the new host', await p.locator('dialog[open] a:has-text("Ouvrir l’hôte")').count() === 1);
{
  const href = await p.locator('dialog[open] a:has-text("Ouvrir l’hôte")').getAttribute('href');
  const hid = new URL(href, BASE).searchParams.get('id');
  const m = await p.evaluate(async (id) => (await (await fetch(`/api/v1/hosts/${id}`)).json()).host.modules, hid);
  check('the modules chosen in the wizard are switched on for the new host', m?.docker === true && m?.openscap === false, JSON.stringify(m));
}
agent.kill();
await p.keyboard.press('Escape');

// ---- status colours: online / offline / needs patch read differently at a glance
await p.goto(`${BASE}/hosts/`); await p.waitForSelector('.tbl');
const colorOf = async (sel) => p.locator(sel).first().evaluate((el) => getComputedStyle(el).color);
const online = await colorOf('.status.tone-ok'), alertc = await colorOf('.status.tone-danger');
check('online and quarantined hosts have distinct status colours (green / red)', online !== alertc, `${online} | ${alertc}`);
check('status pills keep a text label next to the colour', (await p.locator('.status.tone-ok').first().textContent()).trim().length > 0);
check('hosts that need patching carry an amber "à patcher" badge', await p.locator('.tag.tone-warn:has-text("à patcher")').count() >= 1);

// ---- group colours: pick one, see it everywhere, change it from the table
await p.goto(`${BASE}/groups/`); await p.waitForSelector('.tbl, .empty');
await p.click('button:has-text("Nouveau groupe")'); await p.waitForSelector('dialog[open]');
await p.fill('dialog[open] input >> nth=0', 'prod-web'); await p.click('dialog[open] .swatch[aria-label="Prune"]');
check('the dialog previews the chosen colour', await p.locator('dialog[open] .gtag').evaluate((el) => el.style.getPropertyValue('--c').includes('--g-plum')));
await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl .gtag:has-text("prod-web")');
await p.click('tr:has-text("prod-web") button[aria-label="Changer la couleur"]'); await p.click('.popover .swatch[aria-label="Ciel"]');
await p.waitForFunction(() => [...document.querySelectorAll('tr')].find((r) => r.textContent.includes('prod-web'))?.querySelector('.gtag')?.getAttribute('style')?.includes('--g-sky'), null, { timeout: 5000 }).then(() => check('a group colour changes straight from the table', true), () => check('a group colour changes straight from the table', false));
await p.goto(`${BASE}/hosts/`); await p.waitForSelector('.tbl');

// ---- custom group colour from the system colour picker
await p.goto(`${BASE}/groups/`); await p.waitForSelector('.tbl');
await p.click('button:has-text("Nouveau groupe")'); await p.waitForSelector('dialog[open]');
await p.fill('dialog[open] input >> nth=0', 'custom-lab'); await p.locator('dialog[open] input[type=color]').fill('#ff8800');
check('a custom colour previews live in the dialog', await p.locator('dialog[open] .gtag').evaluate((el) => el.style.getPropertyValue('--c') === '#ff8800'));
await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl .gtag:has-text("custom-lab")');
check('the custom colour is saved and drawn in the table', await p.locator('tr:has-text("custom-lab") .gtag').evaluate((el) => el.style.getPropertyValue('--c') === '#ff8800'));
await p.reload(); await p.waitForSelector('.tbl .gtag:has-text("custom-lab")');
check('and survives a reload', await p.locator('tr:has-text("custom-lab") .gtag').evaluate((el) => el.style.getPropertyValue('--c') === '#ff8800'));
await p.click('tr:has-text("custom-lab") button[aria-label="Changer la couleur"]'); await p.locator('.popover input[type=color]').fill('#0044cc');
await p.waitForFunction(() => [...document.querySelectorAll('tr')].find((r) => r.textContent.includes('custom-lab'))?.querySelector('.gtag')?.style.getPropertyValue('--c') === '#0044cc', null, { timeout: 5000 })
  .then(() => check('a custom colour can be set from the table popover', true), () => check('a custom colour can be set from the table popover', false));
await p.goto(`${BASE}/hosts/`); await p.waitForSelector('.tbl');

// ---- the side menu folds away on desktop and its state is remembered
const sideWidth = () => p.locator('.sidebar').evaluate((el) => el.getBoundingClientRect().width);
check('the side menu is open by default on desktop', (await sideWidth()) > 200);
await p.click('.menu-btn'); await p.waitForTimeout(150);
check('the menu button folds the side menu into an icon rail', (await sideWidth()) < 80 && (await p.locator('.menu-btn').getAttribute('aria-expanded')) === 'false', `${await sideWidth()}px`);
check('the rail keeps every entry as an icon with a tooltip and an accessible name', (await p.locator('.sidebar a.navitem .icon').count()) >= 8 && (await p.locator('.sidebar a.navitem[title="Hôtes"]').count()) === 1 && (await p.getByRole('link', { name: 'hôtes' }).count()) >= 1);
check('labels are hidden in the rail', await p.locator('.navitem__label').first().evaluate((el) => el.getBoundingClientRect().width <= 1));
await p.click('.sidebar a.navitem[title="Paquets"]'); await p.waitForURL(/\/packages\/?$/);
check('a rail icon navigates', true);
check('the close button is gone', (await p.locator('.sidebar__close').count()) === 0);
await p.reload(); await p.waitForSelector('.tbl'); await p.waitForTimeout(200);
check('the rail stays after a reload', (await sideWidth()) < 80);
await p.click('.menu-btn'); await p.waitForTimeout(150);
check('and unfolds again', (await sideWidth()) > 200);

// ---- the side menu on a phone: opens, navigates, and closes every way
{
  const m = await open({ width: 390, height: 700 }); await login(m, 'admin', 'e2e-admin-password'); await m.waitForSelector('.menu-btn'); await m.waitForSelector('.stat');
  const shown = () => m.locator('.sidebar').evaluate((el) => el.getBoundingClientRect().right > 100);
  check('phone: the menu starts closed', !(await shown()));
  await m.click('.menu-btn'); await m.waitForTimeout(300); check('phone: the menu button opens it', await shown());
  await m.keyboard.press('Escape'); await m.waitForTimeout(300); check('phone: Escape shuts it', !(await shown()));
  await m.click('.menu-btn'); await m.waitForTimeout(300); await m.mouse.click(370, 400); await m.waitForTimeout(300); check('phone: tapping outside shuts it', !(await shown()));
  await m.click('.menu-btn'); await m.waitForTimeout(300); await m.click('.sidebar a.navitem:has-text("groupes")'); await m.waitForURL(/\/groups\/?$/); await m.waitForTimeout(300);
  check('phone: a menu entry is clickable (nothing covers it) and the menu closes after navigating', !(await shown()));
  await m.context().close();
}

// ---- responsive shell: the sidebar footer and toasts stay on screen at any window size
for (const vp of [{ width: 1280, height: 520 }, { width: 1024, height: 420 }, { width: 800, height: 600 }]) {
  const q = await open(vp); await login(q, 'admin', 'e2e-admin-password'); await q.waitForSelector('.sidebar'); await q.goto(`${BASE}/hosts/`); await q.waitForSelector('.tbl');
  const m = await q.evaluate(() => ({ docScroll: document.documentElement.scrollHeight - innerHeight, user: document.querySelector('.sidebar .usercard')?.getBoundingClientRect().bottom ?? 9999, main: getComputedStyle(document.querySelector('.maincol')).overflowY, h: innerHeight }));
  check(`${vp.width}x${vp.height}: the page frame does not scroll, the content column does`, m.docScroll <= 1 && m.main === 'auto', JSON.stringify(m));
  if (vp.width > 900) check(`${vp.width}x${vp.height}: the sidebar user card is fully visible`, m.user <= m.h + 0.5, `bottom ${m.user} / ${m.h}`);
  await q.goto(`${BASE}/groups/`); await q.waitForSelector('.tbl');
  await q.click('tr:has-text("prod-web") button[aria-label="Changer la couleur"]'); await q.click('.popover .swatch[aria-label="Mousse"]');
  await q.waitForSelector('.toast'); await q.waitForTimeout(600); // let the slide-in animation settle
  const t = await q.locator('.toast').first().boundingBox();
  check(`${vp.width}x${vp.height}: toast notifications are inside the window`, !!t && t.y + t.height <= vp.height && t.x + t.width <= vp.width, JSON.stringify(t));
  await q.context().close();
}

// ---- notifications: a signed webhook receives a real test message
{
  const got = [];
  const sink = http.createServer((req, res) => { let b = ''; req.on('data', (d) => (b += d)); req.on('end', () => { got.push({ h: req.headers, b }); res.end('ok'); }); });
  await new Promise((r) => sink.listen(0, '127.0.0.1', r));
  const hook = `http://127.0.0.1:${sink.address().port}/hook`;
  await p.goto(`${BASE}/notifications/`); await p.waitForSelector('button:has-text("Nouveau canal")');
  await p.click('button:has-text("Nouveau canal")'); await p.waitForSelector('dialog[open]');
  await p.fill('dialog[open] input >> nth=0', 'siem'); await p.fill('dialog[open] input[type=url]', hook); await p.fill('dialog[open] input[type=password]', 'e2e-key');
  await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl tr:has-text("siem")');
  check('a webhook channel is created and its secret is not shown', (await p.locator('.tbl').textContent()).includes(hook) && !(await p.content()).includes('e2e-key'));
  await p.click('tr:has-text("siem") button:has-text("Envoyer un test")'); await p.waitForSelector('.toast');
  await p.waitForTimeout(300);
  const d = got[0];
  const sig = d && 'sha256=' + crypto.createHmac('sha256', 'e2e-key').update(`${d.h['x-croustypatch-timestamp']}.${d.b}`).digest('hex');
  check('"Send test" reaches the webhook with a valid HMAC signature', !!d && d.h['x-croustypatch-signature'] === sig, d ? d.h['x-croustypatch-signature'] : 'nothing received');
  await p.click('[role=tab]:has-text("Règles")'); await p.click('button:has-text("Nouvelle règle")'); await p.waitForSelector('dialog[open]');
  await p.fill('dialog[open] input >> nth=0', 'alertes critiques'); await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl tr:has-text("alertes critiques")');
  check('a rule is saved with the default critical events', (await p.locator('tr:has-text("alertes critiques")').textContent()).includes('quarantaine'));
  await p.click('[role=tab]:has-text("Historique")'); await p.waitForSelector('.tbl, .empty');
  // reports: built from the database and sent even when nothing happened; every attempt is logged
  got.length = 0;
  await p.click('[role=tab]:has-text("Rapports")'); await p.waitForSelector('button:has-text("Nouveau rapport")');
  await p.click('button:has-text("Nouveau rapport")'); await p.waitForSelector('dialog[open] input');
  await p.fill('dialog[open] input >> nth=0', 'Rapport hebdo');
  await p.click('dialog[open] .chip:has-text("siem")');
  await p.click('dialog[open] button:has-text("Prévisualiser")'); await p.waitForSelector('dialog[open] iframe');
  check('the report preview shows the formatted mail', ((await p.locator('dialog[open] iframe').getAttribute('sandbox')) ?? 'x') === '' && (await p.locator('dialog[open] iframe').getAttribute('srcdoc')).includes('Rapport hebdo'));
  await p.keyboard.press('Escape'); await p.waitForTimeout(200);
  await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl tr:has-text("Rapport hebdo")');
  check('a report is saved with its next send time', (await p.locator('tr:has-text("Rapport hebdo")').textContent()).match(/chaque lundi/i) !== null);
  await p.click('tr:has-text("Rapport hebdo") button:has-text("Envoyer maintenant")'); await p.waitForSelector('dialog[open] form');
  await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.alert--ok', { timeout: 15000 });
  await p.waitForTimeout(300);
  const rep = got.map((g) => { try { return JSON.parse(g.b); } catch { return null; } }).find((x) => x?.event === 'report');
  check('"Send now" delivers the report to the channel', !!rep && rep.title.includes('Rapport hebdo') && rep.text.includes('Résumé'), JSON.stringify(rep)?.slice(0, 120));
  await p.click('tr:has-text("Rapport hebdo") button:has-text("Journal")'); await p.waitForSelector('dialog[open] .tbl tr:has-text("envoyé")');
  check('the delivery is logged', true);
  await p.keyboard.press('Escape');
  sink.close();
}

// ---- reboot: forced from the host page
await p.goto(`${BASE}/hosts/`); await p.waitForSelector('.tbl'); await p.click('.tbl tr:has(.status.tone-ok) .cell-host a >> nth=0'); await p.waitForSelector('.tabs');
await p.click('button[aria-label="Plus d’actions"], .page-head__actions .menu button'); await p.click('.menu__item:has-text("Redémarrer")'); await p.waitForSelector('dialog[open]');
{
  const hid = new URL(p.url()).searchParams.get('id');
  const asset = await p.evaluate(async (id) => (await (await fetch(`/api/v1/hosts/${id}`)).json()).host.asset_type, hid);
  const want = asset === 'server' ? '0' : '10';
  check('the reboot warning defaults by host type (server 0 min, workstation/laptop 10 min)', (await p.locator('dialog[open] input[type=number]').inputValue()) === want, `${asset}: ${await p.locator('dialog[open] input[type=number]').inputValue()}`);
}
await p.click('dialog[open] button[type=submit]'); await p.waitForTimeout(1500);
await p.click('[role=tab]:has-text("Historique")'); await p.waitForTimeout(800);
check('the reboot task is created and handled by the agent', await p.locator('.tbl tr:has-text("redémarrage")').count() >= 1);

// ---- personal API token + MCP
await p.goto(`${BASE}/account/`); await p.waitForSelector('button:has-text("Nouveau jeton")');
await p.click('button:has-text("Nouveau jeton")'); await p.waitForSelector('dialog[open]'); await p.fill('dialog[open] input >> nth=0', 'e2e assistant'); await p.click('dialog[open] button[type=submit]');
await p.waitForSelector('code.cs-inline-code'); const tok = (await p.locator('code.cs-inline-code').textContent()).trim();
{
  await p.waitForSelector('.tbl tr:has-text("e2e assistant")', { timeout: 5000 }).catch(() => {});
  const n = await p.locator('.tbl tr:has-text("e2e assistant")').count();
  check('a personal token is shown once after creation', tok.startsWith('cpat_') && n === 1, `${tok.slice(0, 9)}… rows=${n}`);
}
{
  const r = await p.evaluate(async (tk) => (await fetch('/api/v1/mcp', { method: 'POST', credentials: 'omit', headers: { 'content-type': 'application/json', authorization: `Bearer ${tk}` }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'fleet_summary', arguments: {} } }) })).json(), tok);
  check('an MCP client authenticates with the token and reads the fleet', r?.result?.isError === false && typeof r.result.structuredContent.pending_updates === 'number', JSON.stringify(r).slice(0, 120));
}

// ---- compliance tab: an audit can be requested
await p.goto(`${BASE}/hosts/`); await p.waitForSelector('.tbl'); await p.click('.tbl tr:has(.status.tone-ok) .cell-host a >> nth=0'); await p.waitForSelector('.tabs');
await p.waitForSelector('.panel:has-text("Correctifs automatiques")', { timeout: 8000 }).then(() => check('the host page says why a host is, or is not, patched automatically', true), () => check('the host page says why a host is, or is not, patched automatically', false));
await p.waitForSelector('.panel:has-text("Ressources") [role=meter]', { timeout: 40000 }).then(() => check('the host page shows live CPU, memory and disk use reported by the agent', true), () => check('the host page shows live CPU, memory and disk use reported by the agent', false));
check('and the hardware: processor and memory', (await p.locator('.panel:has-text("Système")').first().textContent()).includes('Processeur'));
check('the agent card shows the agent is up to date', await p.locator('.kv .tag:has-text("à jour")').count() === 1);
await p.click('[role=tab]:has-text("Réglages")'); await p.waitForSelector('[role=switch]');
check('the host settings list the modules, with the self-service updates of the app center', await p.locator('[role=switch]').count() === 6);
await p.click('[role=switch]:has-text("OpenSCAP")'); await p.waitForSelector('.toast');
{
  const hid = new URL(p.url()).searchParams.get('id');
  const m = await p.evaluate(async (id) => (await (await fetch(`/api/v1/hosts/${id}`)).json()).host.modules, hid);
  check('a module is switched on for this host only', m?.openscap === true, JSON.stringify(m));
}
await p.click('[role=tab]:has-text("Conformité")'); await p.waitForSelector('button:has-text("Lancer un audit"), .empty');
check('the compliance tab explains how to start when there is no report', (await p.locator('.empty').textContent()).includes('openscap'));
await p.click('button:has-text("Lancer un audit")'); await p.waitForTimeout(1500);
await p.click('[role=tab]:has-text("Historique")'); await p.waitForTimeout(600);
check('a compliance task is created and answered by the agent', await p.locator('.tbl tr:has-text("conformité")').count() >= 1);

// ---- app center: catalog, then self-service install through the agent
{
  // the app center is switched on per host
  await p.evaluate(async () => { for (const h of await (await fetch('/api/v1/hosts')).json()) await fetch(`/api/v1/hosts/${h.id}`, { method: 'PATCH', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ modules: { portal: true } }) }); });
  await p.goto(`${BASE}/portal/`); await p.waitForSelector('[role=tab]:has-text("Catalogue")');
  await p.click('[role=tab]:has-text("Suggestions")'); await p.waitForSelector('.appcard:has-text("htop")');
  check('suggested apps are listed with their packages per system', (await p.locator('.appcard:has-text("htop")').textContent()).includes('pacman'));
  await p.click('.appcard:has-text("htop") button:has-text("Ajouter")'); await p.waitForSelector('.appcard:has-text("htop") .tag:has-text("Dans le catalogue")');
  check('a suggestion is added to the catalog in one click', true);
  await p.click('[role=tab]:has-text("Catalogue")'); await p.click('button:has-text("Nouvelle application")'); await p.waitForSelector('dialog[open]');
  await p.fill('dialog[open] input >> nth=0', 'Outils dev'); await p.fill('dialog[open] input.mono', 'git, make');
  await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl tr:has-text("Outils dev")');
  check('an application is added to the catalog', true);
  await p.click('[role=tab]:has-text("Applications")'); await p.waitForSelector('.appcard:has-text("Outils dev")');
  await p.click('.appcard:has-text("Outils dev") button:has-text("Installer")'); await p.waitForSelector('.toast');
  await p.waitForSelector('.appcard:has-text("Outils dev") .tag:has-text("Installé")', { timeout: 20000 }).then(() => check('a self-service install goes through the agent and shows as installed', true), () => check('a self-service install goes through the agent and shows as installed', false));
}

// ---- light / dark theme
{
  const th = await open(); await login(th, 'admin', 'e2e-admin-password'); await th.waitForSelector('.stat');
  const theme = () => th.evaluate(() => document.documentElement.dataset.theme);
  const bg = () => th.evaluate(() => getComputedStyle(document.body).backgroundColor);
  const before = await theme(), bg0 = await bg();
  await th.click('.themebtn'); await th.waitForTimeout(150);
  check('the theme switch flips light and dark', (await theme()) !== before && (await bg()) !== bg0, `${before} -> ${await theme()}`);
  const shownLogo = () => th.$$eval('.sidebar__brand img', (a) => a.filter((i) => getComputedStyle(i).display !== 'none').map((i) => i.getAttribute('src')));
  const logos = { [await theme()]: await shownLogo() };
  await th.screenshot({ path: `${HERE}/.run/logo-${await theme()}.png`, clip: { x: 0, y: 0, width: 420, height: 140 } });
  await th.reload(); await th.waitForSelector('.stat');
  check('the chosen theme is remembered', (await theme()) !== before);
  await th.click('.themebtn'); await th.waitForTimeout(150);
  logos[await theme()] = await shownLogo();
  await th.screenshot({ path: `${HERE}/.run/logo-${await theme()}.png`, clip: { x: 0, y: 0, width: 420, height: 140 } });
  check('one crab per theme: brick on light, cream on dark', JSON.stringify(logos.light) === JSON.stringify(['/brand/crab-logo.svg']) && JSON.stringify(logos.dark) === JSON.stringify(['/brand/crab-logo-dark.svg']), JSON.stringify(logos));
  await th.context().close();
  const sys = await (await browser.newContext({ ignoreHTTPSErrors: true, colorScheme: 'dark' })).newPage();
  await sys.goto(`${BASE}/login/`); await sys.waitForSelector('form');
  check('with no choice saved, the system dark preference applies', (await sys.evaluate(() => document.documentElement.dataset.theme)) === 'dark');
  await sys.context().close();
}

// ---- 0.7: Docker menu, audit profiles, app icons
{
  const sql = (q) => execFileSync('docker', ['exec', 'croustypatch-pg', 'psql', '-U', 'croustypatch', '-d', 'cp_e2e', '-qAt', '-c', q], { encoding: 'utf8' }).trim();
  const h = sql("select id from hosts order by enrolled_at limit 1");
  sql(`update hosts set modules = modules || '{"docker": true, "openscap": true}' where id='${h}'`);
  sql(`delete from host_packages where host_id='${h}' and source='docker'`);
  sql(`insert into host_packages (host_id,name,source,version,candidate,meta) values ('${h}','docker:site-web','docker','nginx:1.27 @aaaa','nginx:1.27 @bbbb','{"image":"nginx:1.27","project":"site","service":"web","state":"running","health":"healthy"}'), ('${h}','docker:site-db','docker','postgres:17 @cccc',null,'{"image":"postgres:17","project":"site","service":"db","state":"running"}')`);

  // Docker menu
  await p.goto(`${BASE}/`); await p.waitForSelector('.sidebar');
  await p.click('.sidebar a.navitem:has-text("Docker")'); await p.waitForSelector('.tbl tbody tr');
  check('the Docker menu lists every container with its image', (await p.locator('.tbl tbody tr').count()) === 2 && (await p.locator('.tbl').textContent()).includes('nginx:1.27'));
  check('and counts the images that can be updated', (await p.locator('.stat:has-text("Images à mettre à jour")').textContent()).includes('1'));
  await p.click('.chips button:has-text("Mises à jour seulement"), button.chip:has-text("Mises à jour seulement")'); await p.waitForTimeout(500);
  check('only the containers with a newer image can be shown', (await p.locator('.tbl tbody tr').count()) === 1);
  await p.click('button.chip:has-text("Mises à jour seulement")'); await p.fill('input[type=search]', 'postgres'); await p.waitForTimeout(600);
  check('a search matches the image as well as the container name', (await p.locator('.tbl tbody tr').count()) === 1 && (await p.locator('.tbl tbody tr').textContent()).includes('postgres:17'));
  await p.goto(`${BASE}/host/?id=${h}&tab=docker`); await p.waitForSelector('.tbl tbody tr');
  check('each host has its own Docker tab', (await p.locator('.tbl tbody tr').count()) === 2);

  // audit profiles
  sql(`update hosts set facts = jsonb_set(facts, '{openscap}', '{"version":"1.4.2","datastream":"/usr/share/xml/scap/ssg/content/ssg-debian12-ds.xml","profiles":[{"id":"xccdf_org.ssgproject.content_profile_standard","title":"Profil standard"},{"id":"xccdf_org.ssgproject.content_profile_cis","title":"CIS niveau 1"}]}') where id='${h}'`);
  sql(`delete from compliance_reports where host_id='${h}'`);
  sql(`insert into compliance_reports (host_id,profile,datastream,passed,failed,not_applicable,other,score,failed_rules,results_sha256,collected_at) values ('${h}','xccdf_org.ssgproject.content_profile_standard','ds',40,2,3,0,95.2,'[{"rule_id":"xccdf_org.ssgproject.content_rule_package_aide_installed","severity":"low"},{"rule_id":"xccdf_org.ssgproject.content_rule_sysctl_kernel_randomize_va_space","severity":"high"}]','abc',now())`);
  await p.goto(`${BASE}/`); await p.waitForSelector('.sidebar');
  await p.click('.sidebar a.navitem:has-text("Conformité")'); await p.waitForSelector('.stat:has-text("Score de la flotte")');
  check('the Compliance page shows the fleet score and the latest audit of each host', (await p.locator('.stat:has-text("Score de la flotte")').textContent()).includes('95.2') && (await p.locator('.tbl tbody tr').first().textContent()).includes('95.2'));
  await p.click('button:has-text("Nouveau profil")'); await p.waitForSelector('dialog[open]');
  await p.fill('dialog[open] input >> nth=0', 'Postes de travail');
  await p.selectOption('dialog[open] select', { label: 'CIS niveau 1' });
  await p.waitForSelector('dialog[open] button.chip');
  check('the editor offers the rules that fail on the fleet', (await p.locator('dialog[open] button.chip:has-text("sysctl_kernel_randomize_va_space")').count()) === 1 && (await p.locator('dialog[open] button.chip:has-text("package_aide_installed")').count()) === 1);
  await p.click('dialog[open] button.chip:has-text("sysctl_kernel_randomize_va_space")');
  check('a rule is switched off with one click', (await p.locator('dialog[open] .chip--rule').count()) === 1 && (await p.locator('dialog[open] button.chip:has-text("sysctl_kernel_randomize_va_space")').count()) === 0);
  await p.fill('dialog[open] input[placeholder="package_aide_installed"]', 'package_aide_installed'); await p.click('dialog[open] button:has-text("Ajouter")');
  check('or typed by its short name', (await p.locator('dialog[open] .chip--rule').count()) === 2);
  await p.fill('dialog[open] input[placeholder="package_aide_installed"]', 'pas valide'); await p.click('dialog[open] button:has-text("Ajouter")');
  check('an invalid rule name is refused', (await p.locator('dialog[open] .chip--rule').count()) === 2 && (await p.locator('dialog[open] [role=alert], dialog[open] .field__hint:has-text("Lettres")').count()) >= 1);
  await p.fill('dialog[open] input[placeholder="package_aide_installed"]', '');
  await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl tr:has-text("Postes de travail")');
  check('the custom profile is saved with its rules', (await p.locator('.tbl tr:has-text("Postes de travail")').textContent()).includes('2'));
  await p.click('.tbl tr:has-text("Postes de travail") button:has-text("Modifier")'); await p.waitForSelector('dialog[open] .chip--rule');
  await p.click('dialog[open] .chip--rule button >> nth=0');
  check('a rule can be switched back on', (await p.locator('dialog[open] .chip--rule').count()) === 1);
  await p.click('dialog[open] button[type=submit]'); await p.waitForTimeout(700);
  const custom = (await p.evaluate(async () => (await (await fetch('/api/v1/compliance/profiles')).json()).custom))[0];
  check('and the change is kept', custom.name === 'Postes de travail' && custom.disabled_rules.length === 1 && custom.base_profile.endsWith('_profile_cis'));

  // the host picks it, and the audit goes out with the tailoring
  await p.goto(`${BASE}/host/?id=${h}&tab=compliance`); await p.waitForSelector('text=OpenSCAP 1.4.2');
  check('the host tab says OpenSCAP is installed, with its version and profiles', (await p.locator('text=2 profils disponibles').count()) === 1);
  await p.selectOption('select', { label: 'Postes de travail' }); await p.waitForSelector('.toast');
  const own = sql(`select compliance_profile from hosts where id='${h}'`);
  check('the audit profile is chosen per host', own === `custom:${custom.id}`, own);
  await p.click('button:has-text("Lancer un audit")'); await p.waitForTimeout(1200);
  const tail = sql(`select exec->'tailoring'->>'name' || '|' || jsonb_array_length(exec->'tailoring'->'disabled_rules') || '|' || (exec->>'auto_install') from tasks where host_id='${h}' and kind='compliance' order by created_at desc limit 1`);
  check('the audit is sent with the custom profile and the permission to install what is missing', tail === 'Postes de travail|1|true', tail);
  await p.goto(`${BASE}/compliance/`); await p.waitForSelector('.tbl tr:has-text("Postes de travail")');
  await p.click('.tbl tr:has-text("Postes de travail") button:has-text("Supprimer")'); await p.waitForSelector('dialog[open]'); await p.click('dialog[open] button:has-text("Confirmer")');
  await p.waitForSelector('.alert--error', { timeout: 5000 }).then(() => check('a profile that hosts use cannot be deleted', true), () => check('a profile that hosts use cannot be deleted', false));
  sql(`update hosts set compliance_profile='' where id='${h}'`);
  await p.goto(`${BASE}/compliance/`); await p.waitForSelector('.tbl tr:has-text("Postes de travail")');
  await p.click('.tbl tr:has-text("Postes de travail") button:has-text("Supprimer")'); await p.waitForSelector('dialog[open]'); await p.click('dialog[open] button:has-text("Confirmer")');
  await p.waitForSelector('.tbl tr:has-text("Postes de travail")', { state: 'detached', timeout: 5000 }).then(() => check('once nobody uses it, the profile can be deleted', true), () => check('once nobody uses it, the profile can be deleted', false));

  // app icons
  await p.goto(`${BASE}/portal/`); await p.click('[role=tab]:has-text("Suggestions")'); await p.waitForSelector('.appcard:has-text("Firefox")');
  check('suggested apps show their real logo', (await p.locator('.appcard:has-text("Firefox") .appcard__icon--img img').count()) === 1);
  check('and keep a generic icon when they have none', (await p.locator('.appcard:has-text("jq") .appcard__icon--img').count()) === 0 && (await p.locator('.appcard:has-text("jq") .appcard__icon svg').count()) === 1);
  await p.click('[role=tab]:has-text("Catalogue")'); await p.click('button:has-text("Nouvelle application")'); await p.waitForSelector('dialog[open]');
  await p.fill('dialog[open] input >> nth=0', 'Git perso'); await p.fill('dialog[open] input.mono', 'git');
  await p.click('dialog[open] [role=tab]:has-text("Logo")'); await p.fill('dialog[open] input[type=search]', 'git'); await p.waitForTimeout(300);
  await p.click('dialog[open] .iconpick__item[aria-label="Git"]');
  check('a logo is chosen from the gallery and previewed', (await p.locator('dialog[open] .appcard__icon--img img').count()) === 1 && (await p.locator('dialog[open] .iconpick__item[aria-pressed=true]').count()) === 1);
  await p.click('dialog[open] button[type=submit]'); await p.waitForSelector('.tbl tr:has-text("Git perso")');
  await p.click('.tbl tr:has-text("Git perso") button:has-text("Modifier")'); await p.waitForSelector('dialog[open]');
  await p.click('dialog[open] [role=tab]:has-text("Mon image")');
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAEklEQVR4nGM4YaPxHxkzkC4AAJdYIrFAMbGxAAAAAElFTkSuQmCC', 'base64');
  await p.setInputFiles('#icon-file', { name: 'logo.png', mimeType: 'image/png', buffer: png });
  await p.waitForSelector('dialog[open] .appcard__icon--img img');
  await p.setInputFiles('#icon-file', { name: 'logo.gif', mimeType: 'image/gif', buffer: Buffer.from('GIF89a') });
  await p.waitForSelector('dialog[open] [role=alert]');
  check('only PNG, JPEG, WebP and SVG images are accepted', (await p.locator('dialog[open] [role=alert]').textContent()).includes('Format non pris en charge'));
  await p.click('dialog[open] button[type=submit]'); await p.waitForTimeout(800);
  const apps = await p.evaluate(async () => await (await fetch('/api/v1/portal/apps')).json());
  const mine = apps.find((a) => a.name === 'Git perso');
  check('an uploaded image is stored with the app (scaled down, as a data image)', mine?.icon?.startsWith('data:image/png;base64,') && mine.icon.length < 2000, String(mine?.icon).slice(0, 40));
  await p.click('[role=tab]:has-text("Applications")'); await p.waitForSelector('.appcard:has-text("Git perso")');
  check('the app shows its image in the app center', (await p.locator('.appcard:has-text("Git perso") .appcard__icon--img img').count()) === 1);
}

// ---- language + logout
await p.goto(`${BASE}/`); await p.waitForSelector('.stat'); await p.click('.topbar .lang button:has-text("en")'); await p.waitForTimeout(300);
check('language toggle switches the whole interface', await p.locator('.sidebar a:has-text("hosts")').count() === 1 && await p.locator('button:has-text("Add agent")').count() >= 2);
await p.click('.sidebar .usercard'); await p.click('.menu__item:has-text("Sign out")'); await p.waitForURL(/\/login\/?$/);
check('sign out returns to the login page', true);
await p.goto(`${BASE}/hosts/`); await p.waitForURL(/\/login\/?$/); check('protected pages redirect to login after sign out', true);

// ---- a role without enrolment rights
const o = await open(); await login(o, 'op1', 'operator-new-password-1'); await o.waitForSelector('.sidebar');
check('operator does not get an "Add agent" button', await o.locator('button:has-text("Ajouter un agent")').count() === 0);
check('operator does not see account management', await o.locator('.sidebar a:has-text("comptes")').count() === 0);
await o.goto(`${BASE}/hosts/`); await o.waitForSelector('.tbl'); check('operator can still see hosts', (await o.locator('.tbl tbody tr').count()) >= 3);

console.log(res.join('\n'));
const failed = res.filter((r) => r.startsWith('FAIL')).length;
console.log(`\n${res.length - failed} passed, ${failed} failed`);
process.exitCode = failed ? 1 : 0;
await browser.close();
