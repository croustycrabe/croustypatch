'use client';
import { PaperPlaneTilt, Plus } from '@phosphor-icons/react';
import { useState } from 'react';
import { Button, Check, Chip, ConfirmDialog, Dialog, Empty, Field, GroupTag, Notice, PageHeader, Select, Spinner, Table, Tabs, Tag, TextInput, type Tone } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import { useToast } from '@/lib/toast';
import type { HostGroup, Role } from '@/lib/types';

type Kind = 'smtp' | 'webhook' | 'ntfy' | 'gotify';
interface ChannelRow { id: string; name: string; kind: Kind; config: Record<string, unknown>; enabled: boolean; has_secret: boolean }
interface RuleRow { id: string; name: string; enabled: boolean; events: string[]; min_severity: string; frequency: 'instant' | 'digest'; digest_hour: number; group_ids: string[]; channel_ids: string[]; roles: string[]; emails: string[] }
interface Catalog { events: { key: string; severity: string; title_en: string; title_fr: string }[] }
interface Delivery { id: string; event: string; severity: string; title: string; status: 'pending' | 'digest' | 'sent' | 'failed'; attempts: number; last_error: string; created_at: string; sent_at: string | null; channel_id: string }

const SEV_TONE: Record<string, Tone> = { info: 'info', medium: 'neutral', high: 'warn', critical: 'danger' };
const ST_TONE: Record<Delivery['status'], Tone> = { pending: 'neutral', digest: 'info', sent: 'ok', failed: 'danger' };
const KINDS: Kind[] = ['smtp', 'webhook', 'ntfy', 'gotify'];

interface ChannelForm { id?: string; name: string; kind: Kind; url: string; topic: string; host: string; port: number; tls: string; username: string; from: string; to: string; secret: string; has_secret: boolean; enabled: boolean }
const blankChannel = (): ChannelForm => ({ name: '', kind: 'webhook', url: '', topic: '', host: '', port: 587, tls: 'starttls', username: '', from: '', to: '', secret: '', has_secret: false, enabled: true });
const str = (v: unknown) => (typeof v === 'string' ? v : '');
const list = (s: string) => s.split(',').map((x) => x.trim()).filter(Boolean);

function target(c: ChannelRow) {
  if (c.kind === 'smtp') return `${str(c.config.host)}:${String(c.config.port ?? '')}`;
  if (c.kind === 'ntfy') return `${str(c.config.url)}/${str(c.config.topic)}`;
  return str(c.config.url);
}

function Channels({ channels, reload }: { channels: ChannelRow[]; reload: () => void }) {
  const { t } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const act = useAction();
  const [form, setForm] = useState<ChannelForm | null>(null);
  const [deleting, setDeleting] = useState<ChannelRow | null>(null);
  const set = <K extends keyof ChannelForm>(k: K, v: ChannelForm[K]) => setForm((f) => (f ? { ...f, [k]: v } : f));

  const open = (c?: ChannelRow) => {
    act.clearError();
    setForm(c ? { ...blankChannel(), id: c.id, name: c.name, kind: c.kind, enabled: c.enabled, has_secret: c.has_secret, url: str(c.config.url), topic: str(c.config.topic), host: str(c.config.host), port: Number(c.config.port ?? 587), tls: str(c.config.tls) || 'starttls', username: str(c.config.username), from: str(c.config.from), to: Array.isArray(c.config.to) ? (c.config.to as string[]).join(', ') : '' } : blankChannel());
  };
  const save = () => act.run(async () => {
    const f = form!;
    const config = f.kind === 'smtp' ? { host: f.host, port: f.port, tls: f.tls, username: f.username, from: f.from, to: list(f.to) } : f.kind === 'ntfy' ? { url: f.url, topic: f.topic } : { url: f.url };
    const body: Record<string, unknown> = { name: f.name, kind: f.kind, config, enabled: f.enabled };
    if (f.secret || !f.id) body.secret = f.secret;
    await api(f.id ? 'PUT' : 'POST', f.id ? `/notifications/channels/${f.id}` : '/notifications/channels', body);
    setForm(null);
    reload();
    toast.success(t('common.saved'));
  });
  const test = (c: ChannelRow) => act.run(async () => {
    const r = await api<{ ok: boolean; error: string | null }>('POST', `/notifications/channels/${c.id}/test`);
    if (r.ok) toast.success(t('notif.channel.test_ok'));
    else toast.error(t('notif.channel.test_fail', { e: r.error ?? '' }));
  });

  return (
    <div className="stack">
      {can('settings:write') && <div><Button variant="primary" icon={<Plus size={16} weight="bold" />} onClick={() => open()}>{t('notif.channel.new')}</Button></div>}
      {act.error && !form && <Notice kind="error">{act.error}</Notice>}
      <Table head={[t('common.name'), t('notif.channel.kind'), t('notif.channel.target'), t('common.status'), t('common.actions')]} empty={channels.length === 0 && <Empty>{t('notif.channel.none')}</Empty>}>
        {channels.map((c) => (
          <tr key={c.id}>
            <td className="strong">{c.name}</td>
            <td><Tag>{t(`notif.kind.${c.kind}` as MessageKey)}</Tag></td>
            <td className="mono wrap">{target(c)}</td>
            <td><Tag tone={c.enabled ? 'ok' : 'neutral'}>{c.enabled ? t('notif.channel.enabled') : t('users.disabled')}</Tag></td>
            <td>
              {can('settings:write') && (
                <div className="row row--tight">
                  <Button small icon={<PaperPlaneTilt size={16} weight="bold" />} disabled={act.pending} onClick={() => void test(c)}>{t('notif.channel.test')}</Button>
                  <Button small onClick={() => open(c)}>{t('common.edit')}</Button>
                  <Button small onClick={() => setDeleting(c)}>{t('common.delete')}</Button>
                </div>
              )}
            </td>
          </tr>
        ))}
      </Table>

      <Dialog open={!!form} title={form?.id ? t('common.edit') : t('notif.channel.new')} onClose={() => setForm(null)}>
        {form && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <div className="form-grid">
              <Field label={t('common.name')}>{(i) => <TextInput id={i} value={form.name} onChange={(e) => set('name', e.target.value)} required autoFocus />}</Field>
              <Field label={t('notif.channel.kind')}>{(i) => (
                <Select id={i} value={form.kind} disabled={!!form.id} onChange={(e) => set('kind', e.target.value as Kind)}>
                  {KINDS.map((k) => <option key={k} value={k}>{t(`notif.kind.${k}` as MessageKey)}</option>)}
                </Select>)}</Field>
            </div>
            {form.kind === 'smtp' ? (
              <>
                <div className="form-grid">
                  <Field label={t('notif.f.host')}>{(i) => <TextInput id={i} value={form.host} onChange={(e) => set('host', e.target.value)} placeholder="smtp.example.org" required />}</Field>
                  <Field label={t('notif.f.port')}>{(i) => <TextInput id={i} type="number" min={1} max={65535} value={form.port} onChange={(e) => set('port', Number(e.target.value))} />}</Field>
                </div>
                <Field label={t('notif.f.tls')}>{(i) => (
                  <Select id={i} value={form.tls} onChange={(e) => set('tls', e.target.value)}>
                    {['starttls', 'tls', 'none'].map((x) => <option key={x} value={x}>{t(`notif.f.tls.${x}` as MessageKey)}</option>)}
                  </Select>)}</Field>
                <div className="form-grid">
                  <Field label={t('notif.f.username')}>{(i) => <TextInput id={i} value={form.username} onChange={(e) => set('username', e.target.value)} autoComplete="off" />}</Field>
                  <Field label={t('notif.f.from')}>{(i) => <TextInput id={i} type="email" value={form.from} onChange={(e) => set('from', e.target.value)} placeholder="patch@example.org" required />}</Field>
                </div>
                <Field label={t('notif.f.to')}>{(i) => <TextInput id={i} value={form.to} onChange={(e) => set('to', e.target.value)} placeholder="ops@example.org" />}</Field>
              </>
            ) : (
              <div className="form-grid">
                <Field label={t('notif.f.url')}>{(i) => <TextInput id={i} type="url" value={form.url} onChange={(e) => set('url', e.target.value)} placeholder={form.kind === 'ntfy' ? 'https://ntfy.sh' : 'https://'} required />}</Field>
                {form.kind === 'ntfy' && <Field label={t('notif.f.topic')}>{(i) => <TextInput id={i} value={form.topic} onChange={(e) => set('topic', e.target.value)} placeholder="croustypatch" required />}</Field>}
              </div>
            )}
            <Field label={t(`notif.f.secret.${form.kind}` as MessageKey)} hint={form.has_secret ? t('notif.f.secret_keep') : t('notif.f.secret_hint')}>{(i) => <TextInput id={i} type="password" autoComplete="new-password" value={form.secret} onChange={(e) => set('secret', e.target.value)} placeholder={form.has_secret ? '••••••••' : ''} />}</Field>
            {form.kind === 'webhook' && <p className="field__hint">{t('notif.webhook.hint')}</p>}
            <Check checked={form.enabled} onChange={(v) => set('enabled', v)}>{t('notif.channel.enabled')}</Check>
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row"><Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button><Button onClick={() => setForm(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>
      <ConfirmDialog danger message={deleting ? t('notif.channel.confirm_delete') : null} onNo={() => setDeleting(null)} onYes={() => { const c = deleting!; setDeleting(null); void act.run(async () => { await api('DELETE', `/notifications/channels/${c.id}`); reload(); }); }} />
    </div>
  );
}

type RuleForm = Omit<RuleRow, 'id' | 'emails'> & { id?: string; emails: string };
const blankRule = (): RuleForm => ({ name: '', enabled: true, events: ['host.quarantined', 'reboot.overdue', 'task.failed', 'task.rolled_back'], min_severity: 'high', frequency: 'instant', digest_hour: 8, group_ids: [], channel_ids: [], roles: [], emails: '' });
const toggle = (arr: string[], v: string) => (arr.includes(v) ? arr.filter((x) => x !== v) : [...arr, v]);

function Rules({ rules, channels, catalog, reload }: { rules: RuleRow[]; channels: ChannelRow[]; catalog: Catalog; reload: () => void }) {
  const { t, lang } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const act = useAction();
  const groups = useApi<HostGroup[]>('/groups');
  const roles = useApi<Role[]>(can('roles:read') ? '/roles' : null);
  const [form, setForm] = useState<RuleForm | null>(null);
  const [deleting, setDeleting] = useState<RuleRow | null>(null);
  const evName = (k: string) => { const e = catalog.events.find((x) => x.key === k); return e ? (lang === 'fr' ? e.title_fr : e.title_en) : k; };
  const set = <K extends keyof RuleForm>(k: K, v: RuleForm[K]) => setForm((f) => (f ? { ...f, [k]: v } : f));

  const save = () => act.run(async () => {
    const f = form!;
    const body = { ...f, emails: list(f.emails) };
    delete (body as { id?: string }).id;
    await api(f.id ? 'PUT' : 'POST', f.id ? `/notifications/rules/${f.id}` : '/notifications/rules', body);
    setForm(null);
    reload();
    toast.success(t('common.saved'));
  });
  const digestNow = (r: RuleRow) => act.run(async () => {
    const res = await api<{ sent: number }>('POST', `/notifications/rules/${r.id}/digest`);
    toast.success(t('notif.rule.digest_sent', { n: res.sent }));
  });

  return (
    <div className="stack">
      {can('settings:write') && <div><Button variant="primary" icon={<Plus size={16} weight="bold" />} disabled={channels.length === 0} onClick={() => { act.clearError(); setForm({ ...blankRule(), channel_ids: channels.slice(0, 1).map((c) => c.id) }); }}>{t('notif.rule.new')}</Button></div>}
      {act.error && !form && <Notice kind="error">{act.error}</Notice>}
      <Table head={[t('common.name'), t('notif.rule.events'), t('notif.rule.min_severity'), t('notif.rule.frequency'), t('notif.rule.channels'), t('common.actions')]} empty={rules.length === 0 && <Empty>{t('notif.rule.none')}</Empty>}>
        {rules.map((r) => (
          <tr key={r.id}>
            <td className="strong">{r.name}{!r.enabled && <> <Tag>{t('users.disabled')}</Tag></>}</td>
            <td className="wrap"><div className="pkgs">{r.events.slice(0, 3).map((e) => <Tag key={e}>{evName(e)}</Tag>)}{r.events.length > 3 && <Tag>+{r.events.length - 3}</Tag>}</div></td>
            <td><Tag tone={SEV_TONE[r.min_severity]}>{t(`sev.${r.min_severity}` as MessageKey)}</Tag></td>
            <td>{r.frequency === 'digest' ? `${t('notif.rule.digest')} · ${String(r.digest_hour).padStart(2, '0')}:00 UTC` : t('notif.rule.instant')}</td>
            <td className="wrap">{r.channel_ids.map((id) => channels.find((c) => c.id === id)?.name).filter(Boolean).join(', ') || '—'}</td>
            <td>
              {can('settings:write') && (
                <div className="row row--tight">
                  {r.frequency === 'digest' && <Button small disabled={act.pending} onClick={() => void digestNow(r)}>{t('notif.rule.send_digest')}</Button>}
                  <Button small onClick={() => { act.clearError(); setForm({ ...r, emails: r.emails.join(', ') }); }}>{t('common.edit')}</Button>
                  <Button small onClick={() => setDeleting(r)}>{t('common.delete')}</Button>
                </div>
              )}
            </td>
          </tr>
        ))}
      </Table>

      <Dialog wide open={!!form} title={form?.id ? t('common.edit') : t('notif.rule.new')} onClose={() => setForm(null)}>
        {form && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <div className="form-grid">
              <Field label={t('common.name')}>{(i) => <TextInput id={i} value={form.name} onChange={(e) => set('name', e.target.value)} required autoFocus />}</Field>
              <Field label={t('notif.rule.min_severity')}>{(i) => (
                <Select id={i} value={form.min_severity} onChange={(e) => set('min_severity', e.target.value)}>
                  {['info', 'medium', 'high', 'critical'].map((s) => <option key={s} value={s}>{t(`sev.${s}` as MessageKey)}</option>)}
                </Select>)}</Field>
            </div>
            <fieldset className="fieldset">
              <legend>{t('notif.rule.events')}</legend>
              <div className="perm-list">
                {catalog.events.filter((e) => e.key !== 'notification.test').map((e) => (
                  <Check key={e.key} checked={form.events.includes(e.key)} onChange={() => set('events', toggle(form.events, e.key))}>
                    {lang === 'fr' ? e.title_fr : e.title_en} <Tag tone={SEV_TONE[e.severity]}>{t(`sev.${e.severity}` as MessageKey)}</Tag>
                  </Check>
                ))}
              </div>
            </fieldset>
            <div className="form-grid">
              <Field label={t('notif.rule.frequency')}>{(i) => (
                <Select id={i} value={form.frequency} onChange={(e) => set('frequency', e.target.value as RuleForm['frequency'])}>
                  <option value="instant">{t('notif.rule.instant')}</option>
                  <option value="digest">{t('notif.rule.digest')}</option>
                </Select>)}</Field>
              {form.frequency === 'digest' && <Field label={t('notif.rule.digest_hour')}>{(i) => <TextInput id={i} type="number" min={0} max={23} value={form.digest_hour} onChange={(e) => set('digest_hour', Number(e.target.value))} />}</Field>}
            </div>
            <fieldset className="fieldset">
              <legend>{t('notif.rule.channels')}</legend>
              <div className="chips">{channels.map((c) => <Chip key={c.id} active={form.channel_ids.includes(c.id)} onClick={() => set('channel_ids', toggle(form.channel_ids, c.id))}>{c.name}</Chip>)}</div>
            </fieldset>
            <fieldset className="fieldset">
              <legend>{t('notif.rule.scope')}</legend>
              <div className="chips">{groups.data?.map((g) => <GroupTag key={g.id} group={g} active={form.group_ids.includes(g.id)} onClick={() => set('group_ids', toggle(form.group_ids, g.id))} />)}</div>
            </fieldset>
            {roles.data && (
              <fieldset className="fieldset">
                <legend>{t('notif.rule.roles')}</legend>
                <div className="chips">{roles.data.map((r) => <Chip key={r.id} active={form.roles.includes(r.name)} onClick={() => set('roles', toggle(form.roles, r.name))}>{r.name}</Chip>)}</div>
              </fieldset>
            )}
            <Field label={t('notif.rule.emails')}>{(i) => <TextInput id={i} value={form.emails} onChange={(e) => set('emails', e.target.value)} placeholder="secops@example.org" />}</Field>
            <Check checked={form.enabled} onChange={(v) => set('enabled', v)}>{t('notif.channel.enabled')}</Check>
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row"><Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button><Button onClick={() => setForm(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>
      <ConfirmDialog danger message={deleting ? t('notif.rule.confirm_delete') : null} onNo={() => setDeleting(null)} onYes={() => { const r = deleting!; setDeleting(null); void act.run(async () => { await api('DELETE', `/notifications/rules/${r.id}`); reload(); }); }} />
    </div>
  );
}

function History({ channels }: { channels: ChannelRow[] }) {
  const { t, ago } = useI18n();
  const h = useApi<Delivery[]>('/notifications/history', { poll: 15000 });
  if (!h.data) return <Spinner />;
  return (
    <Table head={[t('notif.history.event'), t('notif.rule.channels'), t('notif.history.status'), t('notif.history.attempts'), t('hosts.col.seen')]} empty={h.data.length === 0 && <Empty>{t('notif.history.none')}</Empty>}>
      {h.data.map((d) => (
        <tr key={d.id}>
          <td className="wrap"><Tag tone={SEV_TONE[d.severity]}>{t(`sev.${d.severity}` as MessageKey)}</Tag> {d.title.replace('[CroustyPatch] ', '')}{d.last_error && <span className="sub">{d.last_error}</span>}</td>
          <td>{channels.find((c) => c.id === d.channel_id)?.name ?? '—'}</td>
          <td><Tag tone={ST_TONE[d.status]}>{t(`notif.st.${d.status}` as MessageKey)}</Tag></td>
          <td className="num">{d.attempts}</td>
          <td className="nowrap">{ago(d.sent_at ?? d.created_at)}</td>
        </tr>
      ))}
    </Table>
  );
}

export default function Page() {
  const { t } = useI18n();
  const [tab, setTab] = useState('channels');
  const channels = useApi<ChannelRow[]>('/notifications/channels');
  const rules = useApi<RuleRow[]>('/notifications/rules');
  const catalog = useApi<Catalog>('/notifications/catalog');
  const err = channels.error ?? rules.error ?? catalog.error;
  return (
    <>
      <PageHeader title={t('notif.title')} subtitle={t('notif.subtitle')} />
      {err && <Notice kind="error">{err.message}</Notice>}
      <Tabs value={tab} onChange={setTab} tabs={[
        { id: 'channels', label: t('notif.tab.channels'), count: channels.data?.length || undefined },
        { id: 'rules', label: t('notif.tab.rules'), count: rules.data?.length || undefined },
        { id: 'history', label: t('notif.tab.history') },
      ]} />
      {!channels.data || !rules.data || !catalog.data ? <Spinner /> : (
        <>
          {tab === 'channels' && <Channels channels={channels.data} reload={channels.reload} />}
          {tab === 'rules' && <Rules rules={rules.data} channels={channels.data} catalog={catalog.data} reload={rules.reload} />}
          {tab === 'history' && <History channels={channels.data} />}
        </>
      )}
    </>
  );
}
