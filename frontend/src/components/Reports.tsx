'use client';
import { Eye, PaperPlaneTilt, Plus } from '@phosphor-icons/react';
import { useState } from 'react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import { useToast } from '@/lib/toast';
import type { HostGroup } from '@/lib/types';
import { browserZone } from './PatchSchedule';
import { Button, Check, ConfirmDialog, Dialog, Drawer, Empty, Field, GroupTag, Chip, Notice, Select, Spinner, Table, Tag, TextArea, TextInput } from './ui';

interface Sections { summary: boolean; pending: boolean; history: boolean; history_mode: 'count' | 'days'; history_value: number; failures: boolean; reboots: boolean; patch_days: boolean; docker: boolean }
interface ReportRow {
  id: string; name: string; enabled: boolean; frequency: 'daily' | 'weekly' | 'monthly'; weekday: number; day_of_month: number; send_at: string; timezone: string; language: 'fr' | 'en';
  sections: Sections; group_ids: string[]; emails: string[]; channel_ids: string[]; skip_if_empty: boolean;
  last_run_at: string | null; last_status: '' | 'sent' | 'failed' | 'skipped'; last_error: string; next_run: string | null;
}
interface ChannelLite { id: string; name: string; kind: string; enabled: boolean }
interface Run { at: string; trigger: string; status: 'sent' | 'failed' | 'skipped'; error: string; recipients: string[]; title: string }
type Form = Omit<ReportRow, 'id' | 'last_run_at' | 'last_status' | 'last_error' | 'next_run' | 'emails'> & { id?: string; emails: string };

const SECTION_KEYS: (keyof Sections)[] = ['summary', 'pending', 'history', 'failures', 'reboots', 'patch_days', 'docker'];
const defaultSections = (): Sections => ({ summary: true, pending: true, history: true, history_mode: 'count', history_value: 10, failures: true, reboots: true, patch_days: true, docker: false });
const blank = (lang: 'fr' | 'en'): Form => ({ name: '', enabled: true, frequency: 'weekly', weekday: 0, day_of_month: 1, send_at: '08:00', timezone: browserZone(), language: lang, sections: defaultSections(), group_ids: [], emails: '', channel_ids: [], skip_if_empty: false });
const emails = (s: string) => s.split(/[\s,;]+/).map((x) => x.trim()).filter(Boolean);
const TONE = { sent: 'ok', failed: 'danger', skipped: 'neutral' } as const;

export function Reports({ channels }: { channels: ChannelLite[] }) {
  const { t, tr, date, lang } = useI18n();
  const { can, me } = useAuth();
  const toast = useToast();
  const rows = useApi<ReportRow[]>('/reports', { poll: 20000 });
  const groups = useApi<HostGroup[]>(can('groups:read') ? '/groups' : null);
  const act = useAction();
  const [form, setForm] = useState<Form | null>(null);
  const [deleting, setDeleting] = useState<ReportRow | null>(null);
  const [preview, setPreview] = useState<{ title: string; html: string; text: string; empty: boolean; plain: boolean } | null>(null);
  const [sending, setSending] = useState<{ r: ReportRow; to: string } | null>(null);
  const [log, setLog] = useState<{ r: ReportRow; runs: Run[] } | null>(null);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const writable = can('settings:write');
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setForm((f) => (f ? { ...f, [k]: v } : f));
  const setSection = <K extends keyof Sections>(k: K, v: Sections[K]) => setForm((f) => (f ? { ...f, sections: { ...f.sections, [k]: v } } : f));
  const mailChannels = channels.filter((c) => c.kind === 'smtp');

  const body = (f: Form) => ({ ...f, id: undefined, emails: emails(f.emails), name: f.name.trim() });
  const open = (r?: ReportRow) => { act.clearError(); setForm(r ? { ...r, emails: r.emails.join(', ') } : blank(lang)); };
  const save = () => act.run(async () => {
    const f = form!;
    await api(f.id ? 'PUT' : 'POST', f.id ? `/reports/${f.id}` : '/reports', body(f));
    setForm(null);
    rows.reload();
    toast.success(t('common.saved'));
  });
  const showPreview = () => act.run(async () => {
    const p = await api<{ title: string; html: string; text: string; empty: boolean }>('POST', '/reports/preview', body(form!));
    setPreview({ ...p, plain: false });
  });
  const send = () => act.run(async () => {
    const s = sending!;
    const to = emails(s.to);
    const r = await api<{ status: string; error: string; recipients: string[] }>('POST', `/reports/${s.r.id}/send`, { to });
    setSending(null);
    setResult(r.status === 'sent' ? { ok: true, text: t('rep.sent_to', { to: r.recipients.join(', ') || '—' }) } : { ok: false, text: t('rep.send_failed', { e: r.error }) });
    rows.reload();
  });
  const showLog = (r: ReportRow) => act.run(async () => setLog({ r, runs: await api<Run[]>('GET', `/reports/${r.id}/runs`) }));

  const freq = (r: ReportRow) => {
    const when = `${r.send_at} ${r.timezone}`;
    if (r.frequency === 'daily') return t('rep.freq.daily_at', { when });
    if (r.frequency === 'weekly') return t('rep.freq.weekly_at', { day: tr('dayfull', String(r.weekday)), when });
    return t('rep.freq.monthly_at', { day: r.day_of_month, when });
  };

  return (
    <div className="stack">
      <p className="muted">{t('rep.intro')}</p>
      {writable && <div><Button variant="primary" icon={<Plus size={16} weight="bold" />} onClick={() => open()}>{t('rep.new')}</Button></div>}
      {act.error && !form && !sending && <Notice kind="error">{act.error}</Notice>}
      {result && <Notice kind={result.ok ? 'ok' : 'error'}>{result.text}</Notice>}
      {!rows.data ? <Spinner /> : (
        <Table head={[t('common.name'), t('rep.when'), t('rep.recipients'), t('rep.next'), t('rep.last'), t('common.actions')]} empty={rows.data.length === 0 && <Empty>{t('rep.none')}</Empty>}>
          {rows.data.map((r) => (
            <tr key={r.id}>
              <td className="strong">{r.name}{!r.enabled && <span className="sub">{t('users.disabled')}</span>}</td>
              <td className="wrap">{freq(r)}</td>
              <td className="wrap">{r.emails.length ? r.emails.join(', ') : <span className="muted">—</span>}</td>
              <td className="nowrap">{r.enabled && r.next_run ? date(r.next_run) : '—'}</td>
              <td className="wrap">
                {r.last_status ? <Tag tone={TONE[r.last_status]}>{t(`rep.st.${r.last_status}` as MessageKey)}</Tag> : <span className="muted">—</span>}
                {r.last_run_at && <span className="sub">{date(r.last_run_at)}</span>}
                {r.last_status === 'failed' && r.last_error && <span className="sub" style={{ color: 'var(--danger-text)' }}>{r.last_error}</span>}
              </td>
              <td>
                <div className="row row--tight">
                  {writable && <Button small icon={<PaperPlaneTilt size={16} weight="bold" />} onClick={() => { setResult(null); setSending({ r, to: '' }); }}>{t('rep.send_now')}</Button>}
                  <Button small onClick={() => void showLog(r)}>{t('rep.log')}</Button>
                  {writable && <Button small onClick={() => open(r)}>{t('common.edit')}</Button>}
                  {writable && <Button small onClick={() => setDeleting(r)}>{t('common.delete')}</Button>}
                </div>
              </td>
            </tr>
          ))}
        </Table>
      )}

      <Drawer open={!!form} title={form?.id ? form.name : t('rep.new')} onClose={() => setForm(null)}>
        {form && (
          <form className="stack stack--lg" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <div className="form-grid">
              <Field label={t('common.name')}>{(i) => <TextInput id={i} value={form.name} onChange={(e) => set('name', e.target.value)} required autoFocus />}</Field>
              <Field label={t('rep.language')}>{(i) => <Select id={i} value={form.language} onChange={(e) => set('language', e.target.value as 'fr' | 'en')}><option value="fr">Français</option><option value="en">English</option></Select>}</Field>
            </div>

            <fieldset className="fieldset">
              <legend>{t('rep.schedule')}</legend>
              <div className="form-grid">
                <Field label={t('rep.frequency')}>{(i) => (
                  <Select id={i} value={form.frequency} onChange={(e) => set('frequency', e.target.value as Form['frequency'])}>
                    <option value="daily">{t('rep.f.daily')}</option><option value="weekly">{t('rep.f.weekly')}</option><option value="monthly">{t('rep.f.monthly')}</option>
                  </Select>)}</Field>
                {form.frequency === 'weekly' && <Field label={t('pol.patchday.weekday')}>{(i) => <Select id={i} value={form.weekday} onChange={(e) => set('weekday', Number(e.target.value))}>{[0, 1, 2, 3, 4, 5, 6].map((n) => <option key={n} value={n}>{tr('dayfull', String(n))}</option>)}</Select>}</Field>}
                {form.frequency === 'monthly' && <Field label={t('pol.patchday.day_of_month')}>{(i) => <TextInput id={i} type="number" min={1} max={28} value={form.day_of_month} onChange={(e) => set('day_of_month', Number(e.target.value))} />}</Field>}
                <Field label={t('rep.time')}>{(i) => <TextInput id={i} type="time" value={form.send_at} onChange={(e) => set('send_at', e.target.value)} required />}</Field>
                <Field label={t('pol.patchday.timezone')}>{(i) => <TextInput id={i} value={form.timezone} onChange={(e) => set('timezone', e.target.value)} placeholder="Europe/Paris" required />}</Field>
              </div>
            </fieldset>

            <fieldset className="fieldset">
              <legend>{t('rep.deliver')}</legend>
              <div className="stack">
                <Field label={t('rep.emails')} hint={t('rep.emails_hint')}>{(i) => <TextArea id={i} rows={2} spellCheck={false} value={form.emails} onChange={(e) => set('emails', e.target.value)} placeholder="ops@example.org, boss@example.org" />}</Field>
                <div>
                  <p className="label" style={{ marginBottom: 6 }}>{t('rep.channels')}</p>
                  <div className="chips">
                    {channels.filter((c) => c.enabled).map((c) => <Chip key={c.id} active={form.channel_ids.includes(c.id)} onClick={() => set('channel_ids', form.channel_ids.includes(c.id) ? form.channel_ids.filter((x) => x !== c.id) : [...form.channel_ids, c.id])}>{c.name} · {t(`notif.kind.${c.kind}` as MessageKey)}</Chip>)}
                  </div>
                  {channels.length === 0 && <p className="field__hint" style={{ marginTop: 8 }}>{t('rep.no_channel')}</p>}
                  {mailChannels.length > 0 && !form.channel_ids.some((id) => mailChannels.some((m) => m.id === id)) && emails(form.emails).length > 0 && <p className="field__hint" style={{ marginTop: 8 }}>{t('rep.pick_mail')}</p>}
                </div>
              </div>
            </fieldset>

            <fieldset className="fieldset">
              <legend>{t('rep.content')}</legend>
              <div className="stack">
                <div className="row">
                  {SECTION_KEYS.map((k) => <Check key={k} checked={form.sections[k] as boolean} onChange={(v) => setSection(k, v as never)}>{t(`rep.sec.${k}` as MessageKey)}</Check>)}
                </div>
                {form.sections.history && (
                  <div className="form-grid">
                    <Field label={t('rep.history_mode')}>{(i) => <Select id={i} value={form.sections.history_mode} onChange={(e) => setSection('history_mode', e.target.value as 'count' | 'days')}><option value="count">{t('rep.history.count')}</option><option value="days">{t('rep.history.days')}</option></Select>}</Field>
                    <Field label={t('rep.history_value')}>{(i) => <TextInput id={i} type="number" min={1} max={form.sections.history_mode === 'count' ? 200 : 365} value={form.sections.history_value} onChange={(e) => setSection('history_value', Number(e.target.value))} />}</Field>
                  </div>
                )}
                {(groups.data?.length ?? 0) > 0 && (
                  <div>
                    <p className="label" style={{ marginBottom: 6 }}>{t('rep.groups')}</p>
                    <div className="chips">{groups.data!.map((g) => <GroupTag key={g.id} group={g} active={form.group_ids.includes(g.id)} onClick={() => set('group_ids', form.group_ids.includes(g.id) ? form.group_ids.filter((x) => x !== g.id) : [...form.group_ids, g.id])} />)}</div>
                    <p className="field__hint" style={{ marginTop: 6 }}>{t('rep.groups_hint')}</p>
                  </div>
                )}
                <Check checked={form.skip_if_empty} onChange={(v) => set('skip_if_empty', v)}>{t('rep.skip_empty')}</Check>
                <Check checked={form.enabled} onChange={(v) => set('enabled', v)}>{t('notif.channel.enabled')}</Check>
              </div>
            </fieldset>
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="sticky-actions">
              <Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button>
              <Button icon={<Eye size={16} weight="bold" />} disabled={act.pending} onClick={() => void showPreview()}>{t('rep.preview')}</Button>
              <Button variant="ghost" onClick={() => setForm(null)}>{t('common.cancel')}</Button>
            </div>
          </form>
        )}
      </Drawer>

      <Dialog open={!!preview} title={preview?.title ?? ''} onClose={() => setPreview(null)}>
        {preview && (
          <div className="stack">
            {preview.empty && <Notice>{t('rep.preview_empty')}</Notice>}
            <div className="row"><Chip active={!preview.plain} onClick={() => setPreview({ ...preview, plain: false })}>{t('rep.preview_html')}</Chip><Chip active={preview.plain} onClick={() => setPreview({ ...preview, plain: true })}>{t('rep.preview_text')}</Chip></div>
            {preview.plain
              ? <pre className="cmd" style={{ maxHeight: 420, overflow: 'auto' }}>{preview.text}</pre>
              : <iframe title={preview.title} sandbox="" srcDoc={preview.html} style={{ width: '100%', height: 440, border: '1px solid var(--border)', borderRadius: 8, background: '#fff' }} />}
            <div className="row"><Button onClick={() => setPreview(null)}>{t('common.close')}</Button></div>
          </div>
        )}
      </Dialog>

      <Dialog open={!!sending} title={t('rep.send_now')} onClose={() => setSending(null)}>
        {sending && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void send(); }}>
            <p>{sending.to.trim() ? t('rep.send_only') : t('rep.send_all', { to: sending.r.emails.join(', ') || '—' })}</p>
            <Field label={t('rep.send_to')} hint={t('rep.send_to_hint')}>{(i) => <TextInput id={i} type="email" value={sending.to} onChange={(e) => setSending({ ...sending, to: e.target.value })} placeholder={me?.email ?? 'me@example.org'} autoFocus />}</Field>
            {me?.email && sending.to !== me.email && <div><Button small onClick={() => setSending({ ...sending, to: me.email! })}>{t('rep.send_me', { e: me.email })}</Button></div>}
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row"><Button variant="primary" type="submit" icon={<PaperPlaneTilt size={16} weight="bold" />} disabled={act.pending}>{t('rep.send_now')}</Button><Button onClick={() => setSending(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>

      <Dialog open={!!log} title={log ? t('rep.log_title', { n: log.r.name }) : ''} onClose={() => setLog(null)}>
        {log && (
          <div className="stack">
            <Table head={[t('rep.when_sent'), t('rep.trigger'), t('common.status'), t('rep.recipients')]} empty={log.runs.length === 0 && <Empty>{t('rep.log_none')}</Empty>}>
              {log.runs.map((r, i) => (
                <tr key={i}>
                  <td className="nowrap">{date(r.at)}</td>
                  <td>{t(`rep.trigger.${r.trigger}` as MessageKey)}</td>
                  <td className="wrap"><Tag tone={TONE[r.status]}>{t(`rep.st.${r.status}` as MessageKey)}</Tag>{r.error && <span className="sub" style={{ color: 'var(--danger-text)' }}>{r.error}</span>}</td>
                  <td className="wrap">{r.recipients.join(', ') || '—'}</td>
                </tr>
              ))}
            </Table>
            <div className="row"><Button onClick={() => setLog(null)}>{t('common.close')}</Button></div>
          </div>
        )}
      </Dialog>

      <ConfirmDialog danger message={deleting ? t('rep.confirm_delete') : null} onNo={() => setDeleting(null)} onYes={() => { const r = deleting!; setDeleting(null); void act.run(async () => { await api('DELETE', `/reports/${r.id}`); rows.reload(); }); }} />
    </div>
  );
}
