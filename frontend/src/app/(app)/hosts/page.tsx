'use client';
import { ArrowsClockwise, Desktop, HardDrives, Laptop, X } from '@phosphor-icons/react';
import Link from 'next/link';
import { useSearchParams } from 'next/navigation';
import { Suspense, useMemo, useState } from 'react';
import { AddAgentButton } from '@/components/AddAgent';
import { Button, Chip, ConfirmDialog, Empty, GroupTag, HostStatusLabel, Notice, PageHeader, SearchInput, Select, SortHead, Spinner, Table, Tag } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { HostGroup, HostStatus, HostSummary } from '@/lib/types';

const STATUSES: HostStatus[] = ['online', 'eco', 'maintenance', 'alert', 'quarantined', 'offline', 'pending'];
type SortKey = 'name' | 'status' | 'updates' | 'seen';
const FLAGS = ['updates', 'security', 'attention', 'reboot'] as const;
type Flag = (typeof FLAGS)[number];

function HostIcon({ type }: { type: HostSummary['asset_type'] }) {
  const P = type === 'laptop' ? Laptop : type === 'workstation' ? Desktop : HardDrives;
  return <span className="cell-host__icon"><P size={18} weight="bold" /></span>;
}

function Hosts() {
  const { t, tr, ago } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const params = useSearchParams();
  const [status, setStatus] = useState(params.get('status') ?? '');
  const [group, setGroup] = useState(params.get('group') ?? '');
  const [flags, setFlags] = useState<Set<Flag>>(new Set(FLAGS.filter((f) => params.get(f))));
  const [q, setQ] = useState('');
  const [sort, setSort] = useState<{ key: SortKey; dir: 'asc' | 'desc' }>({ key: 'name', dir: 'asc' });
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const hosts = useApi<HostSummary[]>('/hosts', { poll: 10000 });
  const groups = useApi<HostGroup[]>(can('groups:read') ? '/groups' : null);

  const all = hosts.data ?? [];
  const counts = useMemo(() => {
    const c: Record<string, number> = {};
    for (const h of all) c[h.status] = (c[h.status] ?? 0) + 1;
    return c;
  }, [all]);

  const rows = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const keep = all.filter((h) =>
      (!status || h.status === status) &&
      (!group || h.group_ids.includes(group)) &&
      (!flags.has('updates') || h.updates > 0) &&
      (!flags.has('security') || h.security_updates > 0) &&
      (!flags.has('attention') || ['alert', 'quarantined', 'offline'].includes(h.status)) &&
      (!flags.has('reboot') || h.reboot_required) &&
      (!needle || h.hostname.toLowerCase().includes(needle) || (h.display_name ?? '').toLowerCase().includes(needle) || h.distro.toLowerCase().includes(needle)),
    );
    const dir = sort.dir === 'asc' ? 1 : -1;
    const val = (h: HostSummary): string | number => (sort.key === 'name' ? (h.display_name || h.hostname).toLowerCase() : sort.key === 'status' ? h.status : sort.key === 'updates' ? h.security_updates * 10000 + h.updates : new Date(h.last_seen_at ?? 0).getTime());
    return [...keep].sort((a, b) => (val(a) < val(b) ? -dir : val(a) > val(b) ? dir : 0));
  }, [all, status, group, flags, q, sort]);

  const head = (key: SortKey, label: string) => <SortHead label={label} active={sort.key === key} dir={sort.dir} onClick={() => setSort((s) => (s.key === key ? { key, dir: s.dir === 'asc' ? 'desc' : 'asc' } : { key, dir: key === 'name' ? 'asc' : 'desc' }))} />;
  const allSelected = rows.length > 0 && rows.every((h) => picked.has(h.id));
  const toggleAll = () => setPicked(allSelected ? new Set() : new Set(rows.map((h) => h.id)));
  const toggle = (id: string) => setPicked((p) => { const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n; });

  const bulkPatch = async () => {
    setBusy(true);
    try {
      const withUpdates = rows.filter((h) => picked.has(h.id) && h.updates > 0 && h.status !== 'quarantined').map((h) => h.id);
      const r = await api<{ tasks: unknown[] }>('POST', '/tasks', { host_ids: withUpdates });
      toast.success(t('hosts.bulk.done', { n: r.tasks.length }));
      setPicked(new Set());
      hosts.reload();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t('common.error'));
    } finally {
      setBusy(false);
    }
  };
  const bulkScan = async () => {
    setBusy(true);
    const res = await Promise.allSettled([...picked].map((id) => api('POST', `/hosts/${id}/scan`)));
    toast.success(t('hosts.bulk.done', { n: res.filter((r) => r.status === 'fulfilled').length }));
    setBusy(false);
    setPicked(new Set());
  };

  const flagLabel: Record<Flag, string> = { updates: t('dash.cards.needing'), security: t('dash.cards.security'), attention: t('dash.cards.attention'), reboot: t('attn.reboot') };

  return (
    <>
      <PageHeader title={t('hosts.title')} subtitle={t('hosts.subtitle')} actions={<AddAgentButton />} />
      {hosts.error && <Notice kind="error">{hosts.error.message}</Notice>}
      {!hosts.data ? <Spinner /> : all.length === 0 ? (
        <div className="panel"><Empty title={t('hosts.empty.title')} icon={<HardDrives size={22} weight="bold" />} action={<AddAgentButton />}>{t('hosts.empty.text')}</Empty></div>
      ) : (
        <>
          <div className="toolbar">
            <SearchInput value={q} onChange={setQ} placeholder={t('hosts.search')} />
            {groups.data && groups.data.length > 0 && (
              <Select aria-label={t('nav.groups')} value={group} onChange={(e) => setGroup(e.target.value)} style={{ width: 'auto', minWidth: 170 }}>
                <option value="">{t('hosts.filter.group')}</option>
                {groups.data.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
              </Select>
            )}
            <div className="chips" role="group" aria-label={t('common.status')}>
              <Chip active={status === ''} count={all.length} onClick={() => setStatus('')}>{t('common.all')}</Chip>
              {STATUSES.filter((s) => counts[s]).map((s) => <Chip key={s} active={status === s} count={counts[s]} onClick={() => setStatus(status === s ? '' : s)}>{tr('status', s)}</Chip>)}
            </div>
          </div>
          {flags.size > 0 && (
            <div className="chips" style={{ marginBottom: 12 }}>
              {[...flags].map((f) => <button key={f} type="button" className="tag tag--brick" style={{ cursor: 'pointer' }} onClick={() => setFlags((s) => { const n = new Set(s); n.delete(f); return n; })}>{flagLabel[f]} <X size={11} weight="bold" /></button>)}
            </div>
          )}
          {picked.size > 0 && (
            <div className="alert" style={{ marginBottom: 12, alignItems: 'center' }}>
              <b>{t('hosts.selected', { n: picked.size })}</b>
              <span className="row row--tight right">
                {can('tasks:create') && <Button small variant="primary" disabled={busy} onClick={() => setConfirm(true)}>{t('hosts.bulk.patch')}</Button>}
                {can('tasks:create') && <Button small disabled={busy} icon={<ArrowsClockwise size={14} weight="bold" />} onClick={() => void bulkScan()}>{t('hosts.bulk.scan')}</Button>}
                <Button small variant="ghost" onClick={() => setPicked(new Set())}>{t('hosts.bulk.clear')}</Button>
              </span>
            </div>
          )}
          <Table
            head={[<input key="all" type="checkbox" aria-label={t('hosts.select_all')} checked={allSelected} onChange={toggleAll} style={{ accentColor: 'var(--primary)' }} />, head('name', t('hosts.col.host')), t('hosts.col.group'), t('hosts.col.os'), head('status', t('common.status')), head('updates', t('hosts.col.updates')), head('seen', t('hosts.col.seen'))]}
            empty={rows.length === 0 && <Empty>{t('common.empty')}</Empty>}
          >
            {rows.map((h) => (
              <tr key={h.id}>
                <td><input type="checkbox" aria-label={h.hostname} checked={picked.has(h.id)} onChange={() => toggle(h.id)} style={{ accentColor: 'var(--primary)' }} /></td>
                <td>
                  <div className="cell-host">
                    <HostIcon type={h.asset_type} />
                    <div>
                      <Link href={`/host?id=${h.id}`}>{h.display_name || h.hostname}</Link>
                      <span className="sub">{h.display_name ? h.hostname : tr('asset', h.asset_type)} · {tr('env', h.environment)}</span>
                    </div>
                  </div>
                </td>
                <td className="wrap">{h.group_ids.length ? <div className="pkgs">{h.group_ids.map((g) => { const gr = groups.data?.find((x) => x.id === g); return gr ? <GroupTag key={g} group={gr} /> : <Tag key={g}>…</Tag>; })}</div> : <span className="muted">—</span>}</td>
                <td>{h.distro}<span className="sub">{h.package_manager}{h.reboot_required ? ` · ${t('hosts.reboot')}` : ''}</span></td>
                <td><HostStatusLabel status={h.status} /><span className="sub">{t(h.comm_mode === 'pull' ? 'hosts.mode.pull' : 'hosts.mode.push')}</span></td>
                <td>{h.updates === 0 ? <Tag tone="ok">{t('hosts.up_to_date')}</Tag> : <span className="row row--tight"><Tag tone="warn">{h.updates} {t('hosts.needs_patch')}</Tag>{h.security_updates > 0 && <Tag tone="danger">{h.security_updates} {t('pkg.security')}</Tag>}</span>}</td>
                <td className="nowrap">{ago(h.last_seen_at)}</td>
              </tr>
            ))}
          </Table>
          <p className="field__hint" style={{ marginTop: 10 }}>{rows.length} / {all.length}</p>
        </>
      )}
      <ConfirmDialog message={confirm ? t('host.confirm_patch') : null} onNo={() => setConfirm(false)} onYes={() => { setConfirm(false); void bulkPatch(); }} />
    </>
  );
}

export default function Page() {
  return (
    <>
      <Suspense fallback={<Spinner />}>
        <Hosts />
      </Suspense>
    </>
  );
}
