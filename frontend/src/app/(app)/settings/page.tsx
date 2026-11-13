'use client';
import { useEffect, useState } from 'react';
import { DockerRegistries } from '@/components/DockerRegistries';
import { Button, Card, Field, LinkButton, Notice, PageHeader, Select, Spinner, Switch, Tag, TextArea, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import { useToast } from '@/lib/toast';
import type { Modules, Settings as SettingsT } from '@/lib/types';

const MODULES: (keyof Modules)[] = ['docker', 'helper_script', 'portal', 'notifications', 'eco'];

function Settings() {
  const { t } = useI18n();
  const { can, refresh } = useAuth();
  const toast = useToast();
  const data = useApi<SettingsT>('/settings');
  const [s, setS] = useState<SettingsT | null>(null);
  const act = useAction();
  const writable = can('settings:write');
  // The SSH sources are edited as text (one network per line) and parsed on save.
  const [sources, setSources] = useState<string | null>(null);

  useEffect(() => { if (data.data) setS(data.data); }, [data.data]);
  if (!s) return data.error ? <Notice kind="error">{data.error.message}</Notice> : <Spinner />;

  const save = (key: keyof SettingsT, value: unknown) => act.run(async () => {
    await api('PUT', `/settings/${key}`, value);
    toast.success(t('common.saved'));
    data.reload();
    if (key === 'modules') void refresh();
  });
  const SaveBtn = ({ k }: { k: keyof SettingsT }) => (writable ? <Button variant="primary" small disabled={act.pending} onClick={() => void save(k, s[k])}>{t('common.save')}</Button> : null);

  return (
    <>
      <PageHeader title={t('settings.title')} subtitle={t('settings.subtitle')} />
      <div className="stack stack--lg">
        {act.error && <Notice kind="error">{act.error}</Notice>}

        <Card title={t('settings.modules')} actions={<SaveBtn k="modules" />}>
          <p className="muted" style={{ marginBottom: 16 }}>{t('settings.modules.hint')}</p>
          <div className="stack" style={{ gap: 14 }}>
            {MODULES.map((m) => (
              <div key={m} className="row" style={{ justifyContent: 'space-between', flexWrap: 'nowrap' }}>
                <div>
                  <b>{t(`mod.${m}` as MessageKey)}</b> 
                  <p className="field__hint">{t(`mod.${m}.desc` as MessageKey)}</p>
                </div>
                <Switch checked={!!s.modules[m]} disabled={!writable} onChange={(v) => setS({ ...s, modules: { ...s.modules, [m]: v } })}><span className="sr-only">{t(`mod.${m}` as MessageKey)}</span></Switch>
              </div>
            ))}
          </div>
        </Card>

        <Card title={t('settings.ai')} actions={<SaveBtn k="ai" />}>
          <div className="form-grid">
            <div className="span-all"><Switch checked={s.ai.enabled} disabled={!writable} onChange={(v) => setS({ ...s, ai: { ...s.ai, enabled: v } })}>{t('settings.ai.enabled')}</Switch></div>
            <Field label={t('settings.ai.url')}>{(i) => <TextInput id={i} value={s.ai.base_url} placeholder="http://localhost:11434/v1" disabled={!writable} onChange={(e) => setS({ ...s, ai: { ...s.ai, base_url: e.target.value } })} />}</Field>
            <Field label={t('settings.ai.model')}>{(i) => <TextInput id={i} value={s.ai.model} disabled={!writable} onChange={(e) => setS({ ...s, ai: { ...s.ai, model: e.target.value } })} />}</Field>
            <Field label={t('settings.ai.key')} hint={s.ai.api_key_set ? t('settings.ai.key_set') : undefined}>{(i) => <TextInput id={i} type="password" autoComplete="off" value={s.ai.api_key} disabled={!writable} onChange={(e) => setS({ ...s, ai: { ...s.ai, api_key: e.target.value } })} />}</Field>
          </div>
          <p className="field__hint" style={{ marginTop: 14 }}>{t('settings.ai.note')}</p>
          {writable && <AiTest />}
        </Card>

        <Card title={t('settings.isolation')} actions={writable ? <Button variant="primary" small disabled={act.pending} onClick={() => void save('isolation', { ...s.isolation, ssh_port: Number(s.isolation.ssh_port), ssh_sources: (sources ?? s.isolation.ssh_sources.join('\n')).split(/[\s,]+/).map((x) => x.trim()).filter(Boolean) }).then(() => setSources(null))}>{t('common.save')}</Button> : null}>
          <p className="muted" style={{ marginBottom: 16 }}>{t('settings.isolation.hint')}</p>
          <div className="form-grid">
            <div className="span-all"><Switch checked={s.isolation.keep_ssh} disabled={!writable} onChange={(v) => setS({ ...s, isolation: { ...s.isolation, keep_ssh: v } })}>{t('settings.isolation.keep_ssh')}</Switch></div>
            <Field label={t('settings.isolation.port')}>{(i) => <TextInput id={i} type="number" min={1} max={65535} value={s.isolation.ssh_port} disabled={!writable || !s.isolation.keep_ssh} onChange={(e) => setS({ ...s, isolation: { ...s.isolation, ssh_port: Number(e.target.value) } })} />}</Field>
            <Field label={t('settings.isolation.sources')} hint={t('settings.isolation.sources_hint')}>{(i) => <TextArea id={i} className="input--code" rows={3} spellCheck={false} placeholder="10.0.10.0/24" value={sources ?? s.isolation.ssh_sources.join('\n')} disabled={!writable || !s.isolation.keep_ssh} onChange={(e) => setSources(e.target.value)} />}</Field>
          </div>
        </Card>

        <div className="grid grid--2">
          <Card title={t('settings.conn')} actions={<SaveBtn k="connectivity" />}>
            <Field label={t('settings.conn.mode')} hint={t('settings.conn.hint')}>{(i) => (
              <Select id={i} value={s.connectivity.mode} disabled={!writable} onChange={(e) => setS({ ...s, connectivity: { mode: e.target.value as 'connected' | 'air_gapped' } })}>
                <option value="connected">{t('settings.conn.connected')}</option>
                <option value="air_gapped">{t('settings.conn.air_gapped')}</option>
              </Select>)}</Field>
          </Card>
          <Card title={t('settings.agent')} actions={<SaveBtn k="agent" />}>
            <Field label={t('settings.agent.heartbeat')}>{(i) => <TextInput id={i} type="number" min={10} max={3600} value={s.agent.heartbeat_secs} disabled={!writable} onChange={(e) => setS({ ...s, agent: { heartbeat_secs: Number(e.target.value) } })} />}</Field>
          </Card>
        </div>

        <DockerRegistries mode={data.data?.connectivity.mode} />
      </div>
    </>
  );
}

function AiTest() {
  const { t } = useI18n();
  const act = useAction();
  const [r, setR] = useState<{ ok: boolean; latency_ms: number; contextual_risk?: string; error?: string } | null>(null);
  return (
    <div className="stack" style={{ marginTop: 14 }}>
      <div><Button small disabled={act.pending} onClick={() => void act.run(async () => setR(await api('POST', '/ai/test')))}>{act.pending ? <span className="cs-spinner" aria-hidden="true" /> : null}{t('ai.test')}</Button></div>
      {act.error && <Notice kind="error">{act.error}</Notice>}
      {r && (r.ok ? <Notice kind="ok">{t('ai.test.ok', { ms: r.latency_ms, risk: r.contextual_risk ?? '' })}</Notice> : <Notice kind="error">{t('ai.test.fail', { e: r.error ?? '' })}</Notice>)}
    </div>
  );
}

export default function Page() {
  return <><Settings /></>;
}
