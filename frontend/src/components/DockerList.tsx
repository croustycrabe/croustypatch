'use client';
import { ArrowCircleUp } from '@phosphor-icons/react';
import Link from 'next/link';
import { useState } from 'react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { DockerData, DockerRegistries } from '@/lib/types';
import { useToast } from '@/lib/toast';
import { DockerMigration, type MigrationTarget } from './DockerMigration';
import { Button, Check, Chip, ConfirmDialog, Dialog, Empty, Field, Notice, SearchInput, Select, Spinner, StatCard, Table, Tag } from './ui';

/** Containers with their image state; only the images are ever updated (never start, stop or remove). */
/** The tag of an image reference (`ghcr.io/x/y:v3.3.1` -> `v3.3.1`), `latest` without one. A port is not a tag. */
const tagOf = (image: string) => { const i = image.lastIndexOf(':'); return i > image.lastIndexOf('/') ? image.slice(i + 1) : 'latest'; };
/** The same image with another tag. */
const withTag = (image: string, tag: string) => { const i = image.lastIndexOf(':'); return `${i > image.lastIndexOf('/') ? image.slice(0, i) : image}:${tag}`; };

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
  const [apply, setApply] = useState<DockerData['containers'][number] | null>(null);
  const [ackMajor, setAckMajor] = useState(false);
  const [migrate, setMigrate] = useState<MigrationTarget | null>(null);
  const [registryId, setRegistryId] = useState('');
  const registries = useApi<DockerRegistries>('/docker/registries');
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

  /** Move one container to the suggested version: the agent changes the tag in its compose file and recreates it. */
  const applyVersion = (c: NonNullable<typeof apply>) => act.run(async () => {
    await api('POST', '/tasks', { host_ids: [c.host_id], scope: 'docker', packages: [c.package], to_latest: true, allow_major: !!c.latest?.major && ackMajor, ...(registryId ? { registry_id: registryId } : {}) });
    setApply(null);
    toast.success(t('docker.apply.queued', { name: c.name }));
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
      {registries.data?.blocked && <Notice kind="error">{t('docker.registries.blocked')} <Link href="/settings">{t('nav.settings')}</Link></Notice>}
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
              {!c.open_task && c.check && !c.latest && (c.check.tags.startsWith('error') || c.check.digest.startsWith('error')) && (
                <span style={{ display: 'block', marginBottom: 4 }} title={t('docker.check.error.tip')}>
                  <Tag tone="warn">{t('docker.check.error')}</Tag>
                  <span className="sub">{(c.check.digest.startsWith('error') ? c.check.digest : c.check.tags).replace(/^error: /, '')}</span>
                </span>
              )}
              {c.open_task ? <Tag tone="info">{t('docker.updating')}</Tag> : c.update ? <span className="row row--tight"><Tag tone="warn">{t('docker.available')}</Tag><span className="mono muted">@{c.candidate}</span></span> : !c.latest && <Tag tone="ok">{t('docker.current')}</Tag>}
              {!c.open_task && !c.latest && c.check && ['not_version', 'pinned', 'offline'].includes(c.check.tags) && <span className="sub" style={{ display: 'block' }}>{t(`docker.check.${c.check.tags === 'not_version' ? 'floating' : c.check.tags}` as 'docker.check.pinned')}</span>}
              {c.latest && (
                <span className="sub" style={{ display: 'block' }}>
                  <Tag tone="warn">{t('docker.suggested')}</Tag> <span className="mono">{tagOf(c.image)} → {c.latest.tag}</span>
                  {c.latest.major && <> <Tag tone="warn">{t('docker.major')}</Tag></>}
                  {c.latest.major && c.latest.same_major && <span className="muted"> {t('docker.same_major', { tag: c.latest.same_major })}</span>}
                  {!c.open_task && <span style={{ display: 'block', marginTop: 4 }}><Button small onClick={() => setMigrate({ host_id: c.host_id, hostname: c.hostname, name: c.name, package: c.package, image: c.image, suggested: c.latest?.tag ?? null })}>{t('docker.mig.open')}</Button></span>}
                  {c.open_task ? null : c.latest.apply?.mode === 'file' ? (
                    canUpdate && <span style={{ display: 'block', marginTop: 4 }}><Button small icon={<ArrowCircleUp size={14} weight="bold" />} disabled={act.pending} onClick={() => { act.clearError(); setAckMajor(false); setRegistryId(''); setApply(c); }}>{t('docker.apply.button', { tag: c.latest.tag })}</Button></span>
                  ) : (
                    <span className="muted" style={{ display: 'block' }}>
                      {c.latest.apply?.mode === 'manual' ? t('docker.apply.manual', { reason: c.latest.apply.reason ?? '' }) : t('docker.apply.old_agent')} {t('docker.suggest.how')} <span className="mono">{withTag(c.image, c.latest.tag)}</span>
                    </span>
                  )}
                </span>
              )}
            </td>
          </tr>
        ))}
      </Table>
      <Dialog open={!!apply} title={t('docker.apply.title')} onClose={() => setApply(null)}>
        {apply?.latest && (
          <div className="stack">
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <p style={{ margin: 0 }}><b>{apply.name}</b> <span className="muted">· {apply.hostname}</span></p>
            <p className="mono" style={{ margin: 0 }}>{apply.image} → {withTag(apply.image, apply.latest.tag)}</p>
            <dl className="kv">
              <dt>{t('docker.apply.file')}</dt><dd className="mono wrap">{apply.latest.apply?.file}</dd>
            </dl>
            {registries.data && registries.data.registries.filter((r) => !registries.data!.offline || r.kind === 'local').length > 0 && (
              <Field label={t('docker.registry')} hint={t('docker.registry.hint')}>
                {(i) => (
                  <Select id={i} value={registryId} onChange={(e) => setRegistryId(e.target.value)}>
                    <option value="">{t('docker.registry.keep')}</option>
                    {registries.data!.registries.filter((r) => !registries.data!.offline || r.kind === 'local').map((r) => <option key={r.id} value={r.id}>{r.name} · {r.host}</option>)}
                  </Select>)}
              </Field>
            )}
            {apply.latest.apply?.already && <Notice kind="info">{t('docker.apply.already')}</Notice>}
            <ul className="muted" style={{ margin: 0, paddingLeft: 18 }}>
              <li>{t('docker.apply.what.file')}</li>
              <li>{t('docker.apply.what.recreate')}</li>
              <li>{t('docker.apply.what.rollback')}</li>
            </ul>
            {apply.latest.major && (
              <>
                <Notice kind="error">{t('docker.apply.major_warn')}</Notice>
                <Check checked={ackMajor} onChange={setAckMajor}>{t('docker.apply.major_ack')}</Check>
              </>
            )}
            <div className="row">
              <Button variant="primary" disabled={act.pending || (apply.latest.major && !ackMajor)} onClick={() => void applyVersion(apply)}>{t('docker.apply.go', { tag: apply.latest.tag })}</Button>
              <Button onClick={() => setApply(null)}>{t('common.cancel')}</Button>
            </div>
          </div>
        )}
      </Dialog>
      <DockerMigration target={migrate} onClose={() => setMigrate(null)} onDone={() => data.reload()} />
      <ConfirmDialog message={confirm ? t('docker.confirm', { n: confirm.length }) : null} onNo={() => setConfirm(null)} onYes={() => { if (confirm) void update(confirm); }} />
    </div>
  );
}
