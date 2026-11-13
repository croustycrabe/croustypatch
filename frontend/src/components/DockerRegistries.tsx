'use client';
import { Plus, Trash } from '@phosphor-icons/react';
import { useEffect, useState } from 'react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { DockerRegistries as RegistriesData } from '@/lib/types';
import { Button, Card, Check, Field, Notice, Select, Spinner, Tag, TextInput } from './ui';

interface Row { id: string; name: string; kind: 'local' | 'distant'; host: string; mirrors: string; path_prefix: string; username: string; password: string; password_set: boolean; insecure: boolean }

const blank = (): Row => ({ id: '', name: '', kind: 'local', host: '', mirrors: '', path_prefix: '', username: '', password: '', password_set: false, insecure: false });

/**
 * Settings > Docker registries: where the agents look images up and pull them from. A local registry (on the local
 * network, possibly a pull-through cache of Docker Hub or GHCR) works offline; offline mode cannot do without one.
 */
export function DockerRegistries({ mode }: { mode?: string }) {
  const { t } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const act = useAction();
  const data = useApi<RegistriesData>('/docker/registries');
  const [rows, setRows] = useState<Row[] | null>(null);
  const writable = can('settings:write');

  useEffect(() => {
    if (data.data) setRows(data.data.registries.map((r) => ({ id: r.id, name: r.name, kind: r.kind, host: r.host, mirrors: r.mirror_of.join(', '), path_prefix: r.path_prefix, username: r.username, password: '', password_set: r.password_set, insecure: r.insecure })));
  }, [data.data]);

  // the connectivity mode is saved on the same page: what is said about offline mode follows it
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { data.reload(); }, [mode]);
  const set = (i: number, patch: Partial<Row>) => setRows((rows ?? []).map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const save = () => act.run(async () => {
    await api('PUT', '/docker/registries', { registries: (rows ?? []).map((r) => ({ id: r.id, name: r.name, kind: r.kind, host: r.host, mirror_of: r.mirrors.split(/[\s,;]+/).filter(Boolean), path_prefix: r.path_prefix, username: r.username, password: r.password, insecure: r.insecure })) });
    toast.success(t('common.saved'));
    data.reload();
  });

  const d = data.data;
  return (
    <Card title={t('registries.title')} actions={writable ? <Button variant="primary" small disabled={act.pending || !rows} onClick={() => void save()}>{t('common.save')}</Button> : undefined}>
      <div className="stack">
        <p className="muted" style={{ margin: 0 }}>{t('registries.intro')}</p>
        {d?.blocked && <Notice kind="error">{t('registries.blocked')}</Notice>}
        {d && d.offline && !d.blocked && <Notice kind="info">{t('registries.offline_ok')}</Notice>}
        {act.error && <Notice kind="error">{act.error}</Notice>}
        {!rows ? (data.error ? <Notice kind="error">{data.error.message}</Notice> : <Spinner />) : (
          <>
            {rows.length === 0 && <p className="muted" style={{ margin: 0 }}>{t('registries.none')}</p>}
            {rows.map((r, i) => (
              <div key={r.id || `new-${i}`} className="stack" style={{ border: '1px solid var(--border, #ddd)', borderRadius: 8, padding: 12 }}>
                <div className="form-grid">
                  <Field label={t('registries.name')}>{(id) => <TextInput id={id} value={r.name} maxLength={60} disabled={!writable} onChange={(e) => set(i, { name: e.target.value })} placeholder="Harbor" />}</Field>
                  <Field label={t('registries.kind')}>{(id) => (
                    <Select id={id} value={r.kind} disabled={!writable} onChange={(e) => set(i, { kind: e.target.value as Row['kind'] })}>
                      <option value="local">{t('registries.kind.local')}</option><option value="distant">{t('registries.kind.distant')}</option>
                    </Select>)}</Field>
                  <Field label={t('registries.host')} hint={t('registries.host.hint')}>{(id) => <TextInput id={id} className="mono" value={r.host} disabled={!writable} onChange={(e) => set(i, { host: e.target.value })} placeholder="registry.lan:5000" />}</Field>
                  <Field label={t('registries.mirror')} hint={t('registries.mirror.hint')}>{(id) => <TextInput id={id} className="mono" value={r.mirrors} disabled={!writable} onChange={(e) => set(i, { mirrors: e.target.value })} placeholder="docker.io, ghcr.io" />}</Field>
                  <Field label={t('registries.prefix')} hint={t('registries.prefix.hint')}>{(id) => <TextInput id={id} className="mono" value={r.path_prefix} disabled={!writable} onChange={(e) => set(i, { path_prefix: e.target.value })} placeholder="dockerhub-proxy/" />}</Field>
                  <Field label={t('registries.user')}>{(id) => <TextInput id={id} value={r.username} autoComplete="off" disabled={!writable} onChange={(e) => set(i, { username: e.target.value })} />}</Field>
                  <Field label={t('registries.password')} hint={r.password_set ? t('registries.password.kept') : undefined}>{(id) => <TextInput id={id} type="password" value={r.password} autoComplete="new-password" disabled={!writable || !r.username} onChange={(e) => set(i, { password: e.target.value })} placeholder={r.password_set ? '••••••••' : ''} />}</Field>
                </div>
                <div className="row" style={{ justifyContent: 'space-between' }}>
                  <Check checked={r.insecure} disabled={!writable} onChange={(v) => set(i, { insecure: v })}>{t('registries.insecure')}</Check>
                  {writable && <Button small variant="danger" icon={<Trash size={14} />} onClick={() => setRows(rows.filter((_, j) => j !== i))}>{t('common.delete')}</Button>}
                </div>
                {r.kind === 'local' && <span><Tag tone="ok">{t('registries.works_offline')}</Tag></span>}
              </div>
            ))}
            {writable && rows.length < 20 && <div><Button small icon={<Plus size={14} weight="bold" />} onClick={() => setRows([...rows, blank()])}>{t('registries.add')}</Button></div>}
            <p className="field__hint" style={{ margin: 0 }}>{t('registries.note')}</p>
          </>
        )}
      </div>
    </Card>
  );
}
