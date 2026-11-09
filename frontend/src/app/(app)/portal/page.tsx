'use client';
import { AppWindow, ChartBar, ChatCircle, Code, Cube, Database, FileText, Globe, Image, MusicNotes, Plus, ShieldCheck, Terminal, Wrench, type Icon } from '@phosphor-icons/react';
import { useEffect, useState } from 'react';
import { Button, Check, Chip, ConfirmDialog, Dialog, Empty, Field, GroupTag, Notice, PageHeader, Select, Spinner, Table, Tabs, Tag, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { HostGroup, HostSummary } from '@/lib/types';

interface App { id: string; name: string; version: string; category: string; description: string; icon: string; packages: string[]; group_ids: string[]; enabled: boolean }
interface HostApp extends App { installed: boolean; pending: { task_id: string; status: string; install: boolean } | null }

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
  useEffect(() => { if (!host && hosts.data?.length) setHost(hosts.data.find((h) => h.status === 'online')?.id ?? hosts.data[0]!.id); }, [hosts.data, host]);
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

type Form = Omit<App, 'id' | 'packages'> & { id?: string; packages: string };
const blank = (): Form => ({ name: '', version: '', category: '', description: '', icon: 'app-window', packages: '', group_ids: [], enabled: true });

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
    const body = { ...f, packages: f.packages.split(',').map((x) => x.trim()).filter(Boolean) };
    delete (body as { id?: string }).id;
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
        <Table head={[t('common.name'), t('portal.app.category'), t('portal.app.packages'), t('common.status'), t('common.actions')]} empty={apps.data.length === 0 && <Empty>{t('portal.app.none')}</Empty>}>
          {apps.data.map((a) => (
            <tr key={a.id}>
              <td className="strong">{a.name}{a.version && <span className="sub">{a.version}</span>}</td>
              <td>{a.category || '—'}</td>
              <td className="mono wrap">{a.packages.join(', ')}</td>
              <td><Tag tone={a.enabled ? 'ok' : 'neutral'}>{a.enabled ? t('notif.channel.enabled') : t('users.disabled')}</Tag></td>
              <td><div className="row row--tight"><Button small onClick={() => { act.clearError(); setForm({ ...a, packages: a.packages.join(', ') }); }}>{t('common.edit')}</Button><Button small onClick={() => setDeleting(a)}>{t('common.delete')}</Button></div></td>
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
            <Field label={t('portal.app.packages')} hint={t('portal.app.packages_hint')}>{(i) => <TextInput id={i} className="mono" value={form.packages} onChange={(e) => set('packages', e.target.value)} placeholder="git, make" required />}</Field>
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

export default function Page() {
  const { t } = useI18n();
  const { can } = useAuth();
  const [tab, setTab] = useState('apps');
  return (
    <>
      <PageHeader title={t('portal.title')} subtitle={t('portal.subtitle')} />
      {can('settings:write') && <Tabs value={tab} onChange={setTab} tabs={[{ id: 'apps', label: t('portal.tab.apps') }, { id: 'catalog', label: t('portal.tab.catalog') }]} />}
      {tab === 'apps' ? <Store /> : <Catalog />}
    </>
  );
}
