'use client';
import { ArrowSquareOut, CheckCircle, Copy, Plus, Terminal } from '@phosphor-icons/react';
import Link from 'next/link';
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { MessageKey } from '@/lib/messages';
import type { HostGroup, HostModules, HostSummary } from '@/lib/types';
import { Button, Check, Chip, Drawer, Field, Notice, Section, Select, Skeleton, Tabs, Tag, TextInput } from './ui';

interface Platform { id: string; os: string; arch: string; available: boolean }
interface Created { token: string; details: { expires_at: string } }
interface Form { comm_mode: 'pull' | 'push'; group_id: string; environment: string; label: string; hours: number; max_uses: number; dry_run: boolean; modules: HostModules }

const MODULE_KEYS: (keyof HostModules)[] = ['docker', 'helper_script', 'openscap', 'portal', 'eco'];

const Ctx = createContext<{ open: () => void } | null>(null);

/** Open the "Add agent" wizard from anywhere. */
export function useAddAgent() {
  const c = useContext(Ctx);
  if (!c) throw new Error('useAddAgent outside AddAgentProvider');
  return c;
}

export function AddAgentProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const value = useMemo(() => ({ open: () => setOpen(true) }), []);
  const { t } = useI18n();
  return (
    <Ctx.Provider value={value}>
      {children}
      <Drawer open={open} title={t('agent.title')} subtitle={t('agent.subtitle')} onClose={() => setOpen(false)}>
        {/* mounted only while open, so every opening starts from a clean wizard */}
        {open && <Wizard onClose={() => setOpen(false)} />}
      </Drawer>
    </Ctx.Provider>
  );
}

export function AddAgentButton({ variant = 'primary', small }: { variant?: 'primary' | 'outline'; small?: boolean }) {
  const { t } = useI18n();
  const { can } = useAuth();
  const { open } = useAddAgent();
  if (!can('enrollment:create')) return null;
  return (
    <Button variant={variant} small={small} icon={<Plus size={16} weight="bold" />} onClick={open} title={t('nav.add_agent')}>
      <span className="btn__label">{t('nav.add_agent')}</span>
    </Button>
  );
}

function CommandBox({ text, label }: { text: string; label?: string }) {
  const { t } = useI18n();
  const toast = useToast();
  const copy = () => navigator.clipboard?.writeText(text).then(() => toast.success(t('common.copied')), () => toast.error(t('common.error')));
  return (
    <div className="stack" style={{ gap: 6 }}>
      <div className="row" style={{ justifyContent: 'space-between', flexWrap: 'nowrap' }}>
        <p className="label">{label ?? ''}</p>
        <Button small variant="outline" icon={<Copy size={14} weight="bold" />} onClick={copy}>
          {t('common.copy')}
        </Button>
      </div>
      <pre className="cmd">{text}</pre>
    </div>
  );
}

function Steps({ step }: { step: 0 | 1 | 2 }) {
  const { t } = useI18n();
  const items = [t('agent.step.configure'), t('agent.step.install'), t('agent.step.connected')];
  return (
    <ol className="row" style={{ listStyle: 'none', padding: 0, margin: '0 0 20px', gap: 8 }} aria-label="steps">
      {items.map((label, i) => (
        <li key={label} className="row row--tight" aria-current={i === step ? 'step' : undefined} style={{ opacity: i <= step ? 1 : 0.5 }}>
          <span className="avatar" style={{ width: 24, height: 24, fontSize: 12, background: i <= step ? 'var(--primary)' : undefined, color: i <= step ? 'var(--primary-fg)' : undefined }}>{i < step ? '✓' : i + 1}</span>
          <span className="label">{label}</span>
          {i < 2 && <span className="muted" aria-hidden="true">—</span>}
        </li>
      ))}
    </ol>
  );
}

function Wizard({ onClose }: { onClose: () => void }) {
  const { t, date } = useI18n();
  const { can, modules: defaults } = useAuth();
  const toast = useToast();
  const act = useAction();
  // Pre-filled with the server's defaults for new hosts (Settings > Modules).
  const [f, setF] = useState<Form>(() => ({
    comm_mode: 'pull', group_id: '', environment: 'production', label: '', hours: 24, max_uses: 1, dry_run: false,
    modules: { docker: !!defaults?.docker, helper_script: !!defaults?.helper_script, openscap: !!defaults?.openscap, portal: !!defaults?.portal, eco: defaults?.eco ?? true },
  }));
  const [created, setCreated] = useState<{ token: string; expires_at: string; startedAt: number } | null>(null);
  const [tab, setTab] = useState<'linux' | 'freebsd' | 'manual'>('linux');

  const groups = useApi<HostGroup[]>(can('groups:read') ? '/groups' : null);
  const platforms = useApi<Platform[]>('/agent/platforms');
  // After the command is shown, watch for the machine to enrol.
  const hosts = useApi<HostSummary[]>(created && can('hosts:read') ? '/hosts' : null, { poll: 3000 });
  const arrived = created ? hosts.data?.find((h) => new Date(h.enrolled_at).getTime() >= created.startedAt - 2000) : undefined;

  useEffect(() => {
    if (arrived) toast.success(t('agent.connected', { name: arrived.display_name || arrived.hostname }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [arrived?.id]);

  const origin = typeof window === 'undefined' ? '' : window.location.origin;
  const generate = useCallback(() => act.run(async () => {
    const r = await api<Created>('POST', '/enrollment-tokens', {
      label: f.label || 'Add agent', group_id: f.group_id || null, environment: f.environment, comm_mode: f.comm_mode, hours: Number(f.hours), max_uses: Number(f.max_uses), modules: f.modules,
    });
    setCreated({ token: r.token, expires_at: r.details.expires_at, startedAt: Date.now() });
  }), [act, f]);

  if (!can('enrollment:create')) return <Notice kind="error">{t('common.forbidden')}</Notice>;

  const step: 0 | 1 | 2 = !created ? 0 : arrived ? 2 : 1;
  const missing = (platforms.data ?? []).filter((p) => !p.available);

  if (!created) {
    return (
      <div className="stack stack--lg">
        <Steps step={0} />
        <div className="stack">
          <p className="label">{t('agent.mode')}</p>
          <div className="stack" style={{ gap: 8 }}>
            {(['pull', 'push'] as const).map((m) => (
              <label key={m} className="panel" style={{ padding: '12px 14px', cursor: 'pointer', display: 'flex', gap: 12, alignItems: 'flex-start', borderColor: f.comm_mode === m ? 'var(--primary)' : undefined }}>
                <input type="radio" name="mode" checked={f.comm_mode === m} onChange={() => setF({ ...f, comm_mode: m })} style={{ accentColor: 'var(--primary)', marginTop: 4 }} />
                <span>
                  <b>{t(m === 'pull' ? 'agent.mode.pull' : 'agent.mode.push')}</b>
                  <span className="muted" style={{ display: 'block', fontSize: 12.5 }}>{t(m === 'pull' ? 'agent.mode.pull.hint' : 'agent.mode.push.hint')}</span>
                </span>
              </label>
            ))}
          </div>
        </div>
        <div className="form-grid">
          <Field label={t('agent.group')}>{(id) => (
            <Select id={id} value={f.group_id} onChange={(e) => setF({ ...f, group_id: e.target.value })}>
              <option value="">{t('agent.group.none')}</option>
              {groups.data?.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
            </Select>)}</Field>
          <Field label={t('agent.env')}>{(id) => (
            <Select id={id} value={f.environment} onChange={(e) => setF({ ...f, environment: e.target.value })}>
              {['production', 'staging', 'dev'].map((v) => <option key={v} value={v}>{t(`env.${v}` as 'env.production')}</option>)}
            </Select>)}</Field>
        </div>
        <fieldset className="fieldset">
          <legend>{t('agent.modules')}</legend>
          <p className="field__hint" style={{ marginBottom: 10 }}>{t('agent.modules.hint')}</p>
          <div className="row">
            {MODULE_KEYS.map((m) => <Check key={m} checked={f.modules[m]} onChange={(v) => setF({ ...f, modules: { ...f.modules, [m]: v } })}>{t(`mod.${m}` as MessageKey)}</Check>)}
          </div>
        </fieldset>
        <Section title={t('agent.advanced')}>
          <div className="form-grid">
            <Field label={t('agent.label')}>{(id) => <TextInput id={id} value={f.label} onChange={(e) => setF({ ...f, label: e.target.value })} />}</Field>
            <Field label={t('agent.hours')}>{(id) => <TextInput id={id} type="number" min={1} max={720} value={f.hours} onChange={(e) => setF({ ...f, hours: Number(e.target.value) })} />}</Field>
            <Field label={t('agent.uses')} hint={t('agent.uses.hint')}>{(id) => <TextInput id={id} type="number" min={1} max={1000} value={f.max_uses} onChange={(e) => setF({ ...f, max_uses: Number(e.target.value) })} />}</Field>
          </div>
          <label className="check"><input type="checkbox" checked={f.dry_run} onChange={(e) => setF({ ...f, dry_run: e.target.checked })} /><span>{t('agent.dry_run')}</span></label>
        </Section>
        {act.error && <Notice kind="error">{act.error}</Notice>}
        <div className="row">
          <Button variant="primary" icon={<Terminal size={16} weight="bold" />} disabled={act.pending} onClick={() => void generate()}>{t('agent.generate')}</Button>
          <Button variant="ghost" onClick={onClose}>{t('common.cancel')}</Button>
        </div>
      </div>
    );
  }

  const vars = `CROUSTYPATCH_ENROLL_TOKEN=${created.token}${f.comm_mode === 'push' ? ' CROUSTYPATCH_MODE=push' : ''}${f.dry_run ? ' CROUSTYPATCH_DRY_RUN=1' : ''}`;
  const script = `${origin}/api/v1/agent/install.sh`;
  const manual = [
    `curl -fsSLo croustypatch-agent ${origin}/api/v1/agent/download/linux-x86_64`,
    `curl -fsSL ${origin}/api/v1/agent/download/linux-x86_64.sha256   # must equal: sha256sum croustypatch-agent`,
    `sudo install -m 0755 croustypatch-agent /usr/local/bin/`,
    `sudo env ${vars} croustypatch-agent enroll --server ${origin} --mode ${f.comm_mode}`,
    `sudo croustypatch-agent run`,
  ].join('\n');

  return (
    <div className="stack stack--lg">
      <Steps step={step} />

      {arrived ? (
        <div className="stack">
          <Notice kind="ok">
            <CheckCircle size={20} weight="bold" className="icon" />
            <span><b>{t('agent.connected', { name: arrived.display_name || arrived.hostname })}</b></span>
          </Notice>
          <div className="row">
            <Link className="btn btn--primary" href={`/host?id=${arrived.id}`} onClick={onClose}><ArrowSquareOut size={16} weight="bold" />{t('agent.open_host')}</Link>
            <Button onClick={() => setCreated(null)}>{t('agent.add_another')}</Button>
            <Button variant="ghost" onClick={onClose}>{t('common.close')}</Button>
          </div>
        </div>
      ) : (
        <>
          <Tabs tabs={[{ id: 'linux', label: t('agent.tab.linux') }, { id: 'freebsd', label: t('agent.tab.freebsd') }, { id: 'manual', label: t('agent.tab.manual') }]} value={tab} onChange={setTab} />
          {tab === 'linux' && (
            <div className="stack">
              <CommandBox label={t('agent.run_linux')} text={`curl -fsSL ${script} | sudo env ${vars} sh`} />
              <CommandBox label="wget" text={`wget -qO- ${script} | sudo env ${vars} sh`} />
            </div>
          )}
          {tab === 'freebsd' && <CommandBox label={t('agent.run_freebsd')} text={`fetch -qo - ${script} | env ${vars} sh`} />}
          {tab === 'manual' && (
            <div className="stack">
              <p className="muted">{t('agent.manual.intro')}</p>
              <CommandBox text={manual} />
            </div>
          )}

          <p className="row row--tight muted" aria-live="polite" style={{ fontSize: 13 }}>
            <Skeleton width={10} height={10} /> {t('agent.waiting')}
          </p>
          <p className="field__hint">{t('agent.token_once', { date: date(created.expires_at) })}</p>

          <Section title={t('agent.what')}>
            <ol style={{ margin: 0, paddingLeft: 20 }}>
              <li>{t('agent.what1')}</li>
              <li>{t('agent.what2')}</li>
              <li>{t('agent.what3')}</li>
            </ol>
            <a href="/api/v1/agent/install.sh" target="_blank" rel="noreferrer">{t('agent.view_script')}</a>
          </Section>

          <div className="stack" style={{ gap: 8 }}>
            <p className="label">{t('agent.platforms')}</p>
            <div className="chips">
              {platforms.data?.map((p) => <Tag key={p.id} tone={p.available ? undefined : 'brick'}>{p.id} · {p.available ? '✓' : t('agent.platform.missing')}</Tag>)}
            </div>
            {missing.length > 0 && <p className="field__hint">{t('agent.platform.missing_hint')}</p>}
          </div>
          <div className="row"><Button variant="ghost" onClick={onClose}>{t('common.close')}</Button></div>
        </>
      )}
    </div>
  );
}
