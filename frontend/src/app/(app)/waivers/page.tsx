'use client';
import Link from 'next/link';
import { useState } from 'react';
import { Button, Dialog, Empty, Field, Notice, PageHeader, Select, Spinner, Table, Tag, TextArea, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { HostGroup, HostSummary, Waiver } from '@/lib/types';

function Waivers() {
  const { t, tr, date } = useI18n();
  const { can, me } = useAuth();
  const list = useApi<Waiver[]>('/waivers', { poll: 15000 });
  const hosts = useApi<HostSummary[]>(can('hosts:read') ? '/hosts' : null);
  const groups = useApi<HostGroup[]>(can('groups:read') ? '/groups' : null);
  const [open, setOpen] = useState(false);
  const act = useAction();
  const [f, setF] = useState({ kind: 'host', host_id: '', group_id: '', cve_id: '', package: '', reason: '', controls: '', expires: '' });
  const [local, setLocal] = useState<string | null>(null);

  const create = () => act.run(async () => {
    setLocal(null);
    if (!f.cve_id.trim() && !f.package.trim()) return setLocal(t('waiver.need_one'));
    // The date picker gives a day; the waiver lasts until the end of that day (UTC).
    await api('POST', '/waivers', {
      host_id: f.kind === 'host' ? f.host_id || null : null,
      group_id: f.kind === 'group' ? f.group_id || null : null,
      cve_id: f.cve_id.trim() || null,
      package: f.package.trim() || null,
      reason: f.reason,
      compensating_controls: f.controls,
      expires_at: new Date(`${f.expires}T23:59:59Z`).toISOString(),
    });
    setOpen(false);
    list.reload();
  });
  const post = (id: string, what: 'approve' | 'reject' | 'revoke') => act.run(async () => { await api('POST', `/waivers/${id}/${what}`); list.reload(); });
  const canApprove = can('waivers:approve_ciso') || can('waivers:approve_business');
  const hostName = (id: string | null) => hosts.data?.find((h) => h.id === id)?.hostname ?? id?.slice(0, 8) ?? '—';
  const groupName = (id: string | null) => groups.data?.find((g) => g.id === id)?.name ?? id?.slice(0, 8) ?? '—';
  const tomorrow = new Date(Date.now() + 86400000).toISOString().slice(0, 10);
  const maxDay = new Date(Date.now() + 90 * 86400000).toISOString().slice(0, 10);

  return (
    <>
      <PageHeader title={t('waiver.title')} subtitle={t('waiver.subtitle')}
        actions={can('waivers:create') && <Button variant="primary" onClick={() => { act.clearError(); setLocal(null); setOpen(true); }}>{t('waiver.new')}</Button>} />
      <div className="stack">
        {act.error && !open && <Notice kind="error">{act.error}</Notice>}
        {list.error && <Notice kind="error">{list.error.message}</Notice>}
        {!list.data ? <Spinner /> : (
          <Table head={[t('waiver.target'), t('waiver.cve'), t('waiver.reason'), t('common.status'), t('waiver.expires'), t('common.actions')]} empty={list.data.length === 0 && <Empty>{t('waiver.none')}</Empty>}>
            {list.data.map((w) => (
              <tr key={w.id}>
                <td>{w.host_id ? <Link href={`/host?id=${w.host_id}`}>{hostName(w.host_id)}</Link> : groupName(w.group_id)}<span className="sub">{w.requested_by}</span></td>
                <td className="wrap">{w.cve_id ?? '—'}{w.package && <span className="sub">{w.package}</span>}</td>
                <td className="wrap">{w.reason}<span className="sub">{w.compensating_controls}</span></td>
                <td>
                  <Tag>{tr('waiver.st', w.status)}</Tag>
                  {w.status === 'pending' && (
                    <span className="sub">
                      {t('waiver.ciso')} : {w.ciso_approved_by ?? t('waiver.pending_by')}<br />
                      {t('waiver.business')} : {w.business_approved_by ?? t('waiver.pending_by')}
                    </span>
                  )}
                </td>
                <td className="nowrap">{date(w.expires_at, { dateStyle: 'medium' })}</td>
                <td>
                  <div className="row row--tight">
                    {w.status === 'pending' && canApprove && w.requested_by !== me?.username && w.ciso_approved_by !== me?.username && w.business_approved_by !== me?.username && (
                      <Button small variant="primary" disabled={act.pending} onClick={() => void post(w.id, 'approve')}>{t('waiver.approve')}</Button>
                    )}
                    {w.status === 'pending' && (canApprove || can('waivers:write')) && <Button small disabled={act.pending} onClick={() => void post(w.id, 'reject')}>{t('waiver.reject')}</Button>}
                    {['pending', 'approved'].includes(w.status) && (canApprove || can('waivers:write')) && <Button small disabled={act.pending} onClick={() => void post(w.id, 'revoke')}>{t('waiver.revoke')}</Button>}
                  </div>
                </td>
              </tr>
            ))}
          </Table>
        )}
      </div>
      <Dialog open={open} wide title={t('waiver.new')} onClose={() => setOpen(false)}>
        <form className="form-grid" onSubmit={(e) => { e.preventDefault(); void create(); }}>
          <Field label={t('waiver.target')}>{(i) => <Select id={i} value={f.kind} onChange={(e) => setF({ ...f, kind: e.target.value })}><option value="host">{t('waiver.target.host')}</option><option value="group">{t('waiver.target.group')}</option></Select>}</Field>
          {f.kind === 'host' ? (
            <Field label={t('hosts.col.host')}>{(i) => <Select id={i} value={f.host_id} onChange={(e) => setF({ ...f, host_id: e.target.value })} required><option value="" />{hosts.data?.map((h) => <option key={h.id} value={h.id}>{h.hostname}</option>)}</Select>}</Field>
          ) : (
            <Field label={t('nav.groups')}>{(i) => <Select id={i} value={f.group_id} onChange={(e) => setF({ ...f, group_id: e.target.value })} required><option value="" />{groups.data?.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}</Select>}</Field>
          )}
          <Field label={t('waiver.cve')} hint={t('common.optional')}>{(i) => <TextInput id={i} value={f.cve_id} placeholder="CVE-2026-0001" onChange={(e) => setF({ ...f, cve_id: e.target.value })} />}</Field>
          <Field label={t('waiver.package')} hint={t('common.optional')}>{(i) => <TextInput id={i} value={f.package} onChange={(e) => setF({ ...f, package: e.target.value })} />}</Field>
          <div className="span-all"><Field label={t('waiver.reason')}>{(i) => <TextArea id={i} value={f.reason} minLength={10} onChange={(e) => setF({ ...f, reason: e.target.value })} required />}</Field></div>
          <div className="span-all"><Field label={t('waiver.controls')} hint={t('waiver.controls.hint')}>{(i) => <TextArea id={i} value={f.controls} minLength={10} onChange={(e) => setF({ ...f, controls: e.target.value })} required />}</Field></div>
          <Field label={t('waiver.expires')} hint={t('waiver.expires.hint')}>{(i) => <TextInput id={i} type="date" min={tomorrow} max={maxDay} value={f.expires} onChange={(e) => setF({ ...f, expires: e.target.value })} required />}</Field>
          {(local || act.error) && <div className="span-all"><Notice kind="error">{local ?? act.error}</Notice></div>}
          <div className="span-all row"><Button variant="primary" type="submit" disabled={act.pending}>{t('common.create')}</Button><Button onClick={() => setOpen(false)}>{t('common.cancel')}</Button></div>
        </form>
      </Dialog>
    </>
  );
}

export default function Page() {
  return <><Waivers /></>;
}
