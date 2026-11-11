'use client';
import Link from 'next/link';
import { useSearchParams } from 'next/navigation';
import { Suspense, useState } from 'react';
import { Button, Chip, Drawer, Empty, Notice, PageHeader, SearchInput, Spinner, Table, Tag } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { PackageHost, PackageRow } from '@/lib/types';

function HostsOf({ name, onChanged }: { name: string; onChanged: () => void }) {
  const { t } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const act = useAction();
  const rows = useApi<PackageHost[]>(`/packages/${encodeURIComponent(name)}/hosts`);
  if (!rows.data) return rows.error ? <Notice kind="error">{rows.error.message}</Notice> : <Spinner />;
  const needing = rows.data.filter((h) => h.candidate);
  const updateAll = () => act.run(async () => {
    const r = await api<{ tasks: unknown[] }>('POST', '/tasks', { host_ids: needing.map((h) => h.host_id), packages: [name] });
    toast.success(t('pkg.created', { n: r.tasks.length }));
    rows.reload();
    onChanged();
  });
  return (
    <div className="stack">
      {act.error && <Notice kind="error">{act.error}</Notice>}
      {can('tasks:create') && needing.length > 0 && <div><Button variant="primary" disabled={act.pending} onClick={() => void updateAll()}>{needing.length === 1 ? t('pkg.update_one') : t('pkg.update_all', { n: needing.length })}</Button></div>}
      <Table head={[t('hosts.col.host'), t('host.pkg.installed'), t('host.pkg.candidate')]}>
        {rows.data.map((h) => (
          <tr key={h.host_id}>
            <td><Link href={`/host?id=${h.host_id}`}>{h.hostname}</Link></td>
            <td>{h.version}</td>
            <td>{h.candidate ? <span className="row row--tight">{h.candidate}{h.is_security && <Tag tone="brick">{t('pkg.security')}</Tag>}</span> : <span className="muted">—</span>}</td>
          </tr>
        ))}
      </Table>
    </div>
  );
}

function Packages() {
  const { t } = useI18n();
  const params = useSearchParams();
  const [q, setQ] = useState(params.get('q') ?? '');
  const [updates, setUpdates] = useState(!!params.get('updates'));
  const [open, setOpen] = useState<string | null>(null);
  const list = useApi<PackageRow[]>(`/packages?updates=${updates}&q=${encodeURIComponent(q)}`);

  return (
    <>
      <PageHeader title={t('pkg.title')} subtitle={t('pkg.subtitle')} />
      <div className="toolbar">
        <SearchInput value={q} onChange={setQ} placeholder={t('pkg.search')} />
        <Chip active={updates} onClick={() => setUpdates((v) => !v)}>{t('pkg.updates_only')}</Chip>
      </div>
      {list.error && <Notice kind="error">{list.error.message}</Notice>}
      {list.data && !updates && !q && <p className="muted" style={{ margin: '0 0 8px', fontSize: 13 }}>{t('pkg.all_hint', { n: list.data.length })}</p>}
      {list.data && list.data.length >= 1000 && <Notice>{t('pkg.limit')}</Notice>}
      {!list.data ? <Spinner /> : (
        <Table head={[t('pkg.col.name'), t('pkg.col.hosts'), t('pkg.col.outdated'), t('pkg.col.latest')]} empty={list.data.length === 0 && <Empty>{t('pkg.none')}</Empty>}>
          {list.data.map((p) => (
            <tr key={`${p.source}/${p.name}`} style={{ cursor: 'pointer' }} onClick={() => setOpen(p.name)}>
              <td><button type="button" className="btn btn--ghost btn--sm" style={{ padding: 0, fontWeight: 700 }} onClick={() => setOpen(p.name)}>{p.name}</button> {p.security && <Tag tone="brick">{t('pkg.security')}</Tag>}{p.source !== 'system' && <span className="sub">{p.source}</span>}</td>
              <td>{p.hosts}</td>
              <td>{p.outdated > 0 ? <b>{p.outdated}</b> : <span className="muted">0</span>}</td>
              <td>{p.latest ?? <span className="muted">—</span>}</td>
            </tr>
          ))}
        </Table>
      )}
      <Drawer open={!!open} title={open ?? ''} onClose={() => setOpen(null)}>
        {open && <HostsOf name={open} onChanged={() => void list.reload()} />}
      </Drawer>
    </>
  );
}

export default function Page() {
  return (
    <>
      <Suspense fallback={<Spinner />}>
        <Packages />
      </Suspense>
    </>
  );
}
