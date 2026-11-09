'use client';
import { AppWindow, ChartBar, ChatCircle, Code, Cube, Database, FileText, Globe, Image, MusicNotes, Plus, ShieldCheck, Terminal, Trash, Wrench, type Icon } from '@phosphor-icons/react';
import { useEffect, useState } from 'react';
import { Button, Check, Chip, ConfirmDialog, Dialog, Empty, Field, GroupTag, Notice, PageHeader, Select, Spinner, Table, Tabs, Tag, TextArea, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { HostGroup, HostSummary } from '@/lib/types';

type SystemKey = 'apt' | 'pacman' | 'pkg' | 'dnf' | '*';
interface Command { install: string; uninstall: string; check: string }
type Recipe = { packages: string[] } | { command: Command };
type Recipes = Partial<Record<SystemKey, Recipe>>;
interface App { id: string; name: string; version: string; category: string; description: string; icon: string; recipes: Recipes; group_ids: string[]; enabled: boolean; has_command?: boolean }
interface HostApp extends App { kind: 'packages' | 'command'; packages: string[]; command: Command | null; installed: boolean; pending: { task_id: string; status: string; install: boolean } | null }
interface Suggestion { name: string; category: string; description: string; icon: string; recipes: Recipes; in_catalog: boolean }

const SYSTEMS: SystemKey[] = ['apt', 'pacman', 'pkg', 'dnf', '*'];

const ICONS: Record<string, Icon> = { '': Cube, 'app-window': AppWindow, code: Code, terminal: Terminal, browser: Globe, 'file-text': FileText, 'chart-bar': ChartBar, 'shield-check': ShieldCheck, database: Database, image: Image, 'music-notes': MusicNotes, 'chat-circle': ChatCircle, wrench: Wrench, cube: Cube };

function Store() {
  const { t } = useI18n();
  const toast = useToast();
  const { can } = useAuth();
  const hosts = useApi<HostSummary[]>('/hosts');
  const [host, setHost] = useState('');
  const [cat, setCat] = useState('');
  const view = useApi<{ apps: HostApp[] }>(host ? `/portal/hosts/${host}` : null, { poll: 4000 });
  const act = useAction();
  useEffect(() => { if (!host && hosts.data?.length) setHost((hosts.data.find((h) => h.status === 'online' && h.connected) ?? hosts.data.find((h) => h.status === 'online') ?? hosts.data[0]!).id); }, [hosts.data, host]);
  const go = (a: HostApp, action: 'INSTALL' | 'UNINSTALL') => act.run(async () => {
    await api('POST', '/portal/install', { host_id: host, app_id: a.id, action });
    toast.success(t('portal.requested'));
    view.reload();
  });
  const apps = view.data?.apps ?? [];
  const cats = [...new Set(apps.map((a) => a.category).filter(Boolean))];
  const shown = apps.filter((a) => !cat || a.category === cat);
  return (
    <div className="stack">
      <div className="row">
        <Field label={t('portal.host')}>{(i) => (
          <Select id={i} value={host} onChange={(e) => { setHost(e.target.value); setCat(''); }} style={{ minWidth: 240 }}>
            {!host && <option value="">{t('portal.choose_host')}</option>}
            {hosts.data?.map((h) => <option key={h.id} value={h.id}>{h.display_name || h.hostname}</option>)}
          </Select>)}</Field>
      </div>
      {cats.length > 1 && <div className="chips"><Chip active={!cat} onClick={() => setCat('')}>{t('portal.all')}</Chip>{cats.map((c) => <Chip key={c} active={cat === c} onClick={() => setCat(c)}>{c}</Chip>)}</div>}
      {view.error && <Notice kind="error">{view.error.message.includes('switched off') ? t('portal.off') : view.error.message}</Notice>}
      {act.error && <Notice kind="error">{act.error}</Notice>}
      {host && !view.data && !view.error ? <Spinner /> : shown.length === 0 && !view.error ? <Empty>{t('portal.none')}</Empty> : (
        <div className="appgrid">
          {shown.map((a) => {
            const I = ICONS[a.icon] ?? Cube;
            const busy = !!a.pending;
            return (
              <div key={a.id} className="cs-card appcard">
                <div className="appcard__head"><span className="appcard__icon"><I size={22} weight="bold" /></span><div><b>{a.name}</b><span className="sub">{[a.category, a.version].filter(Boolean).join(' · ')}</span></div></div>
                <p className="appcard__desc">{a.description}</p>
                {a.kind === 'command' && <p className="field__hint mono" title={a.command?.install}>{t('portal.command_badge')} · {a.command?.check}</p>}
                <div className="row row--tight" style={{ marginTop: 'auto' }}>
                  {busy ? <Tag tone="info">{t('portal.working')}</Tag> : a.installed ? <Tag tone="ok">{t('portal.installed')}</Tag> : null}
                  {can('tasks:create') && !busy && (a.installed
                    ? <Button small disabled={act.pending} onClick={() => void go(a, 'UNINSTALL')}>{t('portal.uninstall')}</Button>
                    : <Button small variant="primary" disabled={act.pending} onClick={() => void go(a, 'INSTALL')}>{t('portal.install')}</Button>)}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

interface Row { key: SystemKey; mode: 'packages' | 'command'; packages: string; install: string; uninstall: string; check: string }
type Form = Omit<App, 'id' | 'recipes' | 'has_command'> & { id?: string; rows: Row[] };

const rowsOf = (r: Recipes): Row[] => SYSTEMS.filter((k) => r[k]).map((k) => {
  const x = r[k]!;
  return 'command' in x
    ? { key: k, mode: 'command', packages: '', ...x.command }
    : { key: k, mode: 'packages', packages: x.packages.join(', '), install: '', uninstall: '', check: '' };
});
const recipesOf = (rows: Row[]): Recipes => Object.fromEntries(rows.map((r) => [r.key, r.mode === 'command'
  ? { command: { install: r.install, uninstall: r.uninstall, check: r.check.trim() } }
  : { packages: r.packages.split(/[\s,]+/).map((x) => x.trim()).filter(Boolean) }]));
const newRow = (key: SystemKey): Row => ({ key, mode: 'packages', packages: '', install: '', uninstall: '', check: '' });
const blank = (): Form => ({ name: '', version: '', category: '', description: '', icon: 'app-window', rows: [newRow('*')], group_ids: [], enabled: true });

/** One line per system for the catalog table. */
function RecipeSummary({ recipes }: { recipes: Recipes }) {
  const { t } = useI18n();
  return (
    <div className="stack" style={{ gap: 2 }}>
      {SYSTEMS.filter((k) => recipes[k]).map((k) => {
        const r = recipes[k]!;
        return <span key={k}><b>{k === '*' ? t('portal.recipe.all') : k}</b> <span className="mono">{'command' in r ? `$ ${r.command.install.split('\n')[0]}` : r.packages.join(', ')}</span></span>;
      })}
    </div>
  );
}

function RecipeEditor({ rows, onChange }: { rows: Row[]; onChange: (rows: Row[]) => void }) {
  const { t } = useI18n();
  const { can } = useAuth();
  const mayCommand = can('portal:commands');
  const setRow = (i: number, patch: Partial<Row>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const free = SYSTEMS.filter((k) => !rows.some((r) => r.key === k));
  return (
    <fieldset className="fieldset">
      <legend>{t('portal.app.recipes')}</legend>
      <p className="field__hint" style={{ marginBottom: 12 }}>{t('portal.app.recipes_hint')}</p>
      <div className="stack">
        {rows.map((r, i) => (
          <div key={r.key} className="panel stack" style={{ padding: 12, gap: 10 }}>
            <div className="row" style={{ justifyContent: 'space-between' }}>
              <div className="row row--tight">
                <Select aria-label={t('portal.app.systems')} value={r.key} onChange={(e) => setRow(i, { key: e.target.value as SystemKey })}>
                  {[r.key, ...free].map((k) => <option key={k} value={k}>{k === '*' ? t('portal.recipe.all') : k}</option>)}
                </Select>
                <Chip active={r.mode === 'packages'} onClick={() => setRow(i, { mode: 'packages' })}>{t('portal.recipe.packages')}</Chip>
                <Chip active={r.mode === 'command'} onClick={() => (mayCommand || r.mode === 'command') && setRow(i, { mode: 'command' })}>{t('portal.recipe.command')}</Chip>
              </div>
              <Button small variant="ghost" icon={<Trash size={14} weight="bold" />} onClick={() => onChange(rows.filter((_, j) => j !== i))}>{t('portal.recipe.remove')}</Button>
            </div>
            {r.mode === 'packages' ? (
              <Field label={t('portal.app.packages')}>{(id) => <TextInput id={id} className="mono" value={r.packages} placeholder="git, make" onChange={(e) => setRow(i, { packages: e.target.value })} />}</Field>
            ) : (
              <>
                <Field label={t('portal.recipe.install')} hint={t('portal.recipe.env_hint')}>{(id) => <TextArea id={id} className="input--code" rows={4} spellCheck={false} value={r.install} disabled={!mayCommand} onChange={(e) => setRow(i, { install: e.target.value })} />}</Field>
                <Field label={t('portal.recipe.uninstall')}>{(id) => <TextArea id={id} className="input--code" rows={2} spellCheck={false} value={r.uninstall} disabled={!mayCommand} onChange={(e) => setRow(i, { uninstall: e.target.value })} />}</Field>
                <Field label={t('portal.recipe.check')} hint={t('portal.recipe.check_hint')}>{(id) => <TextInput id={id} className="mono" value={r.check} placeholder="/opt/app/bin/app" disabled={!mayCommand} onChange={(e) => setRow(i, { check: e.target.value })} />}</Field>
              </>
            )}
          </div>
        ))}
        {!mayCommand && <p className="field__hint">{t('portal.recipe.command_perm')}</p>}
        {free.length > 0 && <div><Button small icon={<Plus size={14} weight="bold" />} onClick={() => onChange([...rows, newRow(free[0]!)])}>{t('portal.app.add_system')}</Button></div>}
      </div>
    </fieldset>
  );
}

function Catalog() {
  const { t } = useI18n();
  const toast = useToast();
  const apps = useApi<App[]>('/portal/apps');
  const groups = useApi<HostGroup[]>('/groups');
  const act = useAction();
  const [form, setForm] = useState<Form | null>(null);
  const [deleting, setDeleting] = useState<App | null>(null);
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setForm((f) => (f ? { ...f, [k]: v } : f));
  const save = () => act.run(async () => {
    const f = form!;
    const { id: _id, rows, ...rest } = f;
    const body = { ...rest, recipes: recipesOf(rows) };
    await api(f.id ? 'PUT' : 'POST', f.id ? `/portal/apps/${f.id}` : '/portal/apps', body);
    setForm(null);
    apps.reload();
    toast.success(t('common.saved'));
  });
  return (
    <div className="stack">
      <div><Button variant="primary" icon={<Plus size={16} weight="bold" />} onClick={() => { act.clearError(); setForm(blank()); }}>{t('portal.app.new')}</Button></div>
      {act.error && !form && <Notice kind="error">{act.error}</Notice>}
      {!apps.data ? <Spinner /> : (
        <Table head={[t('common.name'), t('portal.app.category'), t('portal.app.systems'), t('common.status'), t('common.actions')]} empty={apps.data.length === 0 && <Empty>{t('portal.app.none')}</Empty>}>
          {apps.data.map((a) => (
            <tr key={a.id}>
              <td className="strong">{a.name}{a.version && <span className="sub">{a.version}</span>}</td>
              <td>{a.category || '—'}</td>
              <td className="wrap"><RecipeSummary recipes={a.recipes} /></td>
              <td><Tag tone={a.enabled ? 'ok' : 'neutral'}>{a.enabled ? t('notif.channel.enabled') : t('users.disabled')}</Tag></td>
              <td><div className="row row--tight"><Button small onClick={() => { act.clearError(); const { recipes, has_command: _hc, ...rest } = a; setForm({ ...rest, rows: rowsOf(recipes) }); }}>{t('common.edit')}</Button><Button small onClick={() => setDeleting(a)}>{t('common.delete')}</Button></div></td>
            </tr>
          ))}
        </Table>
      )}
      <Dialog open={!!form} title={form?.id ? t('common.edit') : t('portal.app.new')} onClose={() => setForm(null)}>
        {form && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <div className="form-grid">
              <Field label={t('common.name')}>{(i) => <TextInput id={i} value={form.name} onChange={(e) => set('name', e.target.value)} required autoFocus />}</Field>
              <Field label={t('portal.app.version')}>{(i) => <TextInput id={i} value={form.version} onChange={(e) => set('version', e.target.value)} />}</Field>
              <Field label={t('portal.app.category')}>{(i) => <TextInput id={i} value={form.category} onChange={(e) => set('category', e.target.value)} placeholder="Développement" />}</Field>
              <Field label={t('portal.app.icon')}>{(i) => <Select id={i} value={form.icon} onChange={(e) => set('icon', e.target.value)}>{Object.keys(ICONS).filter(Boolean).map((k) => <option key={k} value={k}>{k}</option>)}</Select>}</Field>
            </div>
            <Field label={t('common.description')}>{(i) => <TextInput id={i} value={form.description} onChange={(e) => set('description', e.target.value)} />}</Field>
            <RecipeEditor rows={form.rows} onChange={(rows) => set('rows', rows)} />
            <fieldset className="fieldset"><legend>{t('portal.app.scope')}</legend><div className="chips">{groups.data?.map((g) => <GroupTag key={g.id} group={g} active={form.group_ids.includes(g.id)} onClick={() => set('group_ids', form.group_ids.includes(g.id) ? form.group_ids.filter((x) => x !== g.id) : [...form.group_ids, g.id])} />)}</div></fieldset>
            <Check checked={form.enabled} onChange={(v) => set('enabled', v)}>{t('notif.channel.enabled')}</Check>
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row"><Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button><Button onClick={() => setForm(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>
      <ConfirmDialog danger message={deleting ? t('portal.app.confirm_delete') : null} onNo={() => setDeleting(null)} onYes={() => { const a = deleting!; setDeleting(null); void act.run(async () => { await api('DELETE', `/portal/apps/${a.id}`); apps.reload(); }); }} />
    </div>
  );
}

function Suggestions() {
  const { t } = useI18n();
  const toast = useToast();
  const list = useApi<Suggestion[]>('/portal/suggestions');
  const act = useAction();
  const [q, setQ] = useState('');
  const [cat, setCat] = useState('');
  const add = (s: Suggestion) => act.run(async () => {
    await api('POST', '/portal/apps', { name: s.name, category: s.category, description: s.description, icon: s.icon, recipes: s.recipes, group_ids: [], enabled: true });
    toast.success(t('portal.sugg.added', { name: s.name }));
    list.reload();
  });
  if (!list.data) return list.error ? <Notice kind="error">{list.error.message}</Notice> : <Spinner />;
  const cats = [...new Set(list.data.map((s) => s.category))].sort();
  const needle = q.trim().toLowerCase();
  const shown = list.data.filter((s) => (!cat || s.category === cat) && (!needle || `${s.name} ${s.description}`.toLowerCase().includes(needle)));
  return (
    <div className="stack">
      <p className="muted">{t('portal.sugg.sub')}</p>
      <div className="row">
        <Field label={t('portal.sugg.search')}>{(i) => <TextInput id={i} value={q} onChange={(e) => setQ(e.target.value)} style={{ minWidth: 240 }} />}</Field>
      </div>
      <div className="chips"><Chip active={!cat} onClick={() => setCat('')}>{t('portal.all')}</Chip>{cats.map((c) => <Chip key={c} active={cat === c} onClick={() => setCat(c)}>{c}</Chip>)}</div>
      {act.error && <Notice kind="error">{act.error}</Notice>}
      <div className="appgrid">
        {shown.map((s) => {
          const I = ICONS[s.icon] ?? Cube;
          return (
            <div key={s.name} className="cs-card appcard">
              <div className="appcard__head"><span className="appcard__icon"><I size={22} weight="bold" /></span><div><b>{s.name}</b><span className="sub">{s.category}</span></div></div>
              <p className="appcard__desc">{s.description}</p>
              <div className="field__hint"><RecipeSummary recipes={s.recipes} /></div>
              <div className="row row--tight" style={{ marginTop: 'auto' }}>
                {s.in_catalog ? <Tag tone="ok">{t('portal.sugg.in_catalog')}</Tag> : <Button small variant="primary" icon={<Plus size={14} weight="bold" />} disabled={act.pending} onClick={() => void add(s)}>{t('portal.sugg.add')}</Button>}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export default function Page() {
  const { t } = useI18n();
  const { can } = useAuth();
  const [tab, setTab] = useState('apps');
  return (
    <>
      <PageHeader title={t('portal.title')} subtitle={t('portal.subtitle')} />
      {can('settings:write') && <Tabs value={tab} onChange={setTab} tabs={[{ id: 'apps', label: t('portal.tab.apps') }, { id: 'catalog', label: t('portal.tab.catalog') }, { id: 'suggestions', label: t('portal.tab.suggestions') }]} />}
      {tab === 'apps' ? <Store /> : tab === 'catalog' ? <Catalog /> : <Suggestions />}
    </>
  );
}
