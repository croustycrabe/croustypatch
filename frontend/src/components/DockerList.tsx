'use client';
import { ArrowCircleUp } from '@phosphor-icons/react';
import Link from 'next/link';
import { useState } from 'react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { DockerData } from '@/lib/types';
import { useToast } from '@/lib/toast';
import { Button, Chip, ConfirmDialog, Empty, Notice, SearchInput, Spinner, StatCard, Table, Tag } from './ui';

/** Containers with their image state; only the images are ever updated (never start, stop or remove). */
export function DockerList({ hostId }: { hostId?: string }) {
  const { t, date } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const act = useAction();
  const [q, setQ] = useState('');
  const [updatesOnly, setUpdatesOnly] = useState(false);
  const [newerOnly, setNewerOnly] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState<string[] | null>(null);
  const data = useApi<DockerData>(`/docker?updates=${updatesOnly}&newer=${newerOnly}&q=${encodeURIComponent(q)}${hostId ? `&host=${hostId}` : ''}`, { poll: 20000 });
  const d = data.data;
  const key = (c: { host_id: string; package: string }) => `${c.host_id}|${c.package}`;

  const update = (keys: string[]) => act.run(async () => {
    // one request per host: the packages are that host's containers
    const byHost = new Map<string, string[]>();
    for (const k of keys) { const [h, p] = k.split('|') as [string, string]; byHost.set(h, [...(byHost.get(h) ?? []), p]); }
    let n = 0;
    for (const [h, packages] of byHost) {
      const r = await api<{ tasks: unknown[] }>('POST', '/tasks', { host_ids: [h], packages });
      n += r.tasks.length;
    }
    setConfirm(null);
    setPicked(new Set());
    toast.success(t('docker.queued', { n }));
    data.reload();
  });

  if (!d) return data.error ? <Notice kind="error">{data.error.message}</Notice> : <Spinner />;
  const updatable = d.containers.filter((c) => c.update && !c.open_task);
  const canUpdate = can('tasks:create');

  return (
    <div className="stack">
      {!hostId && (
        <div className="grid grid--stats">
          <StatCard value={d.summary.containers} label={t('docker.stat.containers')} />
          <StatCard value={d.summary.with_updates} tone={d.summary.with_updates > 0 ? 'warn' : 'ok'} label={t('docker.stat.updates')} />
          <StatCard value={d.summary.with_newer_version} label={t('docker.stat.newer')} />
          <StatCard value={d.summary.hosts} label={t('docker.stat.hosts')} />
        </div>
      )}
      <p className="muted" style={{ margin: 0 }}>{t('docker.note')}{d.summary.with_newer_version > 0 ? ` ${t('docker.newer.note')}` : ''}</p>
      <div className="toolbar">
        <SearchInput value={q} onChange={setQ} placeholder={t('docker.search')} />
        <Chip active={updatesOnly} onClick={() => setUpdatesOnly((v) => !v)}>{t('docker.updates_only')}</Chip>
        <Chip active={newerOnly} onClick={() => setNewerOnly((v) => !v)}>{t('docker.newer_only')}</Chip>
        {canUpdate && (
          <span className="row row--tight right">
            {updatable.length > 0 && <Button small onClick={() => setPicked(new Set(updatable.map(key)))}>{t('docker.pick_all', { n: updatable.length })}</Button>}
            <Button small variant="primary" icon={<ArrowCircleUp size={16} weight="bold" />} disabled={picked.size === 0 || act.pending} onClick={() => setConfirm([...picked])}>{t('docker.update_selected')}{picked.size ? ` (${picked.size})` : ''}</Button>
          </span>
        )}
      </div>
      {act.error && !confirm && <Notice kind="error">{act.error}</Notice>}
      <Table head={[canUpdate ? '' : null, t('docker.col.container'), hostId ? null : t('hosts.col.host'), t('docker.col.image'), t('docker.col.state'), t('docker.col.update')].filter((h) => h !== null)} empty={d.containers.length === 0 && <Empty>{t(updatesOnly || q ? 'docker.none_match' : 'docker.none')}</Empty>}>
        {d.containers.map((c) => (
          <tr key={key(c)}>
            {canUpdate && <td>{c.update && !c.open_task && <input type="checkbox" aria-label={c.name} checked={picked.has(key(c))} onChange={(e) => { const n = new Set(picked); if (e.target.checked) n.add(key(c)); else n.delete(key(c)); setPicked(n); }} style={{ accentColor: 'var(--primary)' }} />}</td>}
            <td className="strong wrap">{c.name}{(c.project || c.service) && <span className="sub">{[c.project, c.service].filter(Boolean).join(' / ')}</span>}</td>
            {!hostId && <td><Link href={`/host?id=${c.host_id}&tab=docker`}>{c.hostname}</Link></td>}
            <td className="mono wrap">{c.image}<span className="sub">{c.current ? `@${c.current}` : ''}</span></td>
            <td className="wrap">
              {c.state ? <Tag tone={c.state === 'running' ? 'ok' : 'warn'}>{c.state}</Tag> : <span className="muted">—</span>}
              {c.health && <Tag tone={c.health === 'healthy' ? 'ok' : c.health === 'unhealthy' ? 'danger' : 'neutral'}>{c.health}</Tag>}
              {c.started_at && c.state === 'running' && <span className="sub">{t('docker.since', { d: date(c.started_at, { dateStyle: 'medium' }) })}</span>}
            </td>
            <td className="wrap">
              {c.open_task ? <Tag tone="info">{t('docker.updating')}</Tag> : c.update ? <span className="row row--tight"><Tag tone="warn">{t('docker.available')}</Tag><span className="mono muted">@{c.candidate}</span></span> : <Tag tone="ok">{t('docker.current')}</Tag>}
              {c.latest && (
                <span className="sub" style={{ display: 'block' }}>
                  <Tag tone="info">{t('docker.newer', { tag: c.latest.tag })}</Tag>
                  {c.latest.major && <Tag tone="warn">{t('docker.major')}</Tag>}
                  {c.latest.major && c.latest.same_major && <span className="muted"> {t('docker.same_major', { tag: c.latest.same_major })}</span>}
                </span>
              )}
            </td>
          </tr>
        ))}
      </Table>
      <ConfirmDialog message={confirm ? t('docker.confirm', { n: confirm.length }) : null} onNo={() => setConfirm(null)} onYes={() => { if (confirm) void update(confirm); }} />
    </div>
  );
}
