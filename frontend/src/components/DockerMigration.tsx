'use client';
import { ArrowDown, ArrowUp, MagicWand, Play, Plus, Trash } from '@phosphor-icons/react';
import { useEffect, useState } from 'react';
import { api } from '@/lib/api';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { MigrationInfo, MigrationPlan, ShellStep, Task, TaskEvent } from '@/lib/types';
import { Button, ConfirmDialog, Dialog, Field, Notice, Select, Spinner, Tag, TextArea, TextInput } from './ui';

export interface MigrationTarget { host_id: string; hostname: string; name: string; package: string; image: string; suggested: string | null }

const TERMINAL = ['succeeded', 'failed', 'rolled_back', 'expired', 'cancelled', 'quarantined'];

/**
 * The migration console of a container: prebuilt command lines for its kind of software (PostgreSQL major upgrade,
 * MariaDB, Redis…), an AI draft when a provider is configured, and a run on the host with its output followed live.
 * The lines are always editable and always read by a person before they run as root.
 */
export function DockerMigration({ target, onClose, onDone }: { target: MigrationTarget | null; onClose: () => void; onDone: () => void }) {
  const { t, tr } = useI18n();
  const act = useAction();
  const ai = useAction();
  const [to, setTo] = useState('');
  const [toSent, setToSent] = useState('');
  const [steps, setSteps] = useState<ShellStep[]>([]);
  const [warnings, setWarnings] = useState<string[][]>([]);
  const [source, setSource] = useState<'manual' | 'ai'>('manual');
  const [plan, setPlan] = useState<MigrationPlan | null>(null);
  const [tplId, setTplId] = useState('');
  const [confirm, setConfirm] = useState(false);
  const [taskId, setTaskId] = useState<string | null>(null);

  const open = !!target;
  const info = useApi<MigrationInfo>(target ? `/docker/migration?host_id=${target.host_id}&container=${encodeURIComponent(target.package)}&to=${encodeURIComponent(toSent)}` : null);
  const task = useApi<{ task: Task; events: TaskEvent[] }>(taskId ? `/tasks/${taskId}` : null, { poll: 3000 });

  // a new container: start from its suggested tag, with nothing in the editor
  useEffect(() => {
    if (!target) return;
    setTo(target.suggested ?? ''); setToSent(target.suggested ?? ''); setSteps([]); setWarnings([]); setSource('manual'); setPlan(null); setTplId(''); setTaskId(null);
    act.clearError(); ai.clearError();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target?.host_id, target?.package]);
  // the tag is sent a moment after typing stops
  useEffect(() => { const id = setTimeout(() => setToSent(to.trim()), 500); return () => clearTimeout(id); }, [to]);
  useEffect(() => { if (info.data && !tplId && info.data.templates.length) setTplId(info.data.templates[0]!.id); }, [info.data, tplId]);

  const d = info.data;
  const tpl = d?.templates.find((x) => x.id === tplId);
  const running = !!taskId && !TERMINAL.includes(task.data?.task.status ?? '');
  const finished = !!taskId && !running && !!task.data;
  const editable = !!d?.can_run && !taskId;
  const load = (list: ShellStep[], src: 'manual' | 'ai' = 'manual', warn: string[][] = []) => { setSteps(list.map((s) => ({ ...s }))); setWarnings(warn); setSource(src); setPlan(null); };
  const set = (i: number, patch: Partial<ShellStep>) => setSteps(steps.map((s, j) => (j === i ? { ...s, ...patch } : s)));
  const move = (i: number, by: number) => {
    const j = i + by;
    if (j < 0 || j >= steps.length) return;
    const n = [...steps]; [n[i], n[j]] = [n[j]!, n[i]!]; setSteps(n);
    const w = [...warnings]; [w[i], w[j]] = [w[j] ?? [], w[i] ?? []]; setWarnings(w);
  };
  const remove = (i: number) => { setSteps(steps.filter((_, j) => j !== i)); setWarnings(warnings.filter((_, j) => j !== i)); };

  const draft = () => ai.run(async () => {
    if (!target) return;
    const p = await api<MigrationPlan>('POST', '/docker/migration-plan', { host_id: target.host_id, container: target.package, to: toSent });
    load(p.steps.map(({ title, command }) => ({ title, command })), 'ai', p.steps.map((s) => s.warnings));
    setPlan(p);
  });
  const run = () => act.run(async () => {
    if (!target) return;
    const r = await api<{ task_id: string }>('POST', '/docker/shell', { host_id: target.host_id, container: target.package, steps, source });
    setConfirm(false);
    setTaskId(r.task_id);
  });
  const close = () => { if (finished) onDone(); onClose(); };
  const invalid = steps.length === 0 || steps.some((s) => !s.title.trim() || !s.command.trim());

  return (
    <>
    <Dialog open={open} title={t('docker.mig.title')} onClose={close} wide>
      {target && (
        <div className="stack">
          <p style={{ margin: 0 }}><b>{target.name}</b> <span className="muted">· {target.hostname}</span> <span className="mono muted">· {target.image}</span></p>
          {!d ? (info.error ? <Notice kind="error">{info.error.message}</Notice> : <Spinner />) : (
            <>
              {!d.docker_module && <Notice kind="error">{t('docker.mig.no_module')}</Notice>}
              {d.docker_module && !d.agent_ready && <Notice kind="error">{t('docker.mig.old_agent')}</Notice>}
              {!d.can_run && <Notice kind="info">{t('docker.mig.no_perm')}</Notice>}
              {d.problem && <Notice kind="info">{d.problem}</Notice>}
              <Notice kind="info">{t('docker.mig.warn')}</Notice>

              <div className="form-grid">
                <Field label={t('docker.mig.to')} hint={d.suggested ? t('docker.mig.to.hint', { tag: d.suggested }) : undefined}>
                  {(i) => <TextInput id={i} className="mono" value={to} disabled={!!taskId} onChange={(e) => setTo(e.target.value)} placeholder="18-alpine" />}
                </Field>
                <Field label={t('docker.mig.template')}>
                  {(i) => (
                    <Select id={i} value={tplId} disabled={!!taskId || d.templates.length === 0} onChange={(e) => setTplId(e.target.value)}>
                      {d.templates.map((x) => <option key={x.id} value={x.id}>{x.title}</option>)}
                    </Select>)}
                </Field>
              </div>
              {tpl && <p className="muted" style={{ margin: 0 }}>{tpl.description}</p>}
              <div className="row">
                <Button small disabled={!tpl || !!taskId} onClick={() => tpl && load(tpl.steps)}>{t('docker.mig.use')}</Button>
                <Button small disabled={!tpl || tpl.rollback.length === 0 || !!taskId} onClick={() => tpl && load(tpl.rollback)}>{t('docker.mig.use_rollback')}</Button>
                {d.ai.available && d.can_run && (
                  <Button small variant="primary" icon={<MagicWand size={14} weight="bold" />} disabled={ai.pending || !!taskId || !d.templates.length} onClick={() => void draft()}>{ai.pending ? t('docker.mig.ai.working') : t('docker.mig.ai')}</Button>
                )}
              </div>
              {d.ai.available && d.can_run && <p className="field__hint" style={{ margin: 0 }}>{t('docker.mig.ai.hint')}</p>}
              {ai.error && <Notice kind="error">{ai.error}</Notice>}
              {plan && (
                <Notice kind="info">
                  <b>{t('docker.mig.ai.draft', { model: plan.model })}</b> {plan.summary}
                  {plan.risks.length > 0 && <ul style={{ margin: '6px 0 0', paddingLeft: 18 }}>{plan.risks.map((r, i) => <li key={i}>{r}</li>)}</ul>}
                </Notice>
              )}

              <section className="stack">
                <h3 className="panel__title">{t('docker.mig.steps', { n: steps.length })}</h3>
                {steps.length === 0 && <p className="muted" style={{ margin: 0 }}>{t('docker.mig.empty')}</p>}
                {steps.map((s, i) => (
                  <div key={i} className="stack" style={{ borderLeft: '3px solid var(--border, #ccc)', paddingLeft: 10 }}>
                    <div className="row" style={{ flexWrap: 'nowrap' }}>
                      <span className="muted" style={{ minWidth: 22 }}>{i + 1}.</span>
                      <TextInput value={s.title} maxLength={120} disabled={!editable} onChange={(e) => set(i, { title: e.target.value })} aria-label={t('docker.mig.step_title', { n: i + 1 })} />
                      {editable && (
                        <span className="row row--tight" style={{ flexWrap: 'nowrap' }}>
                          <Button small aria-label={t('docker.mig.up')} disabled={i === 0} icon={<ArrowUp size={14} />} onClick={() => move(i, -1)} />
                          <Button small aria-label={t('docker.mig.down')} disabled={i === steps.length - 1} icon={<ArrowDown size={14} />} onClick={() => move(i, 1)} />
                          <Button small variant="danger" aria-label={t('common.delete')} icon={<Trash size={14} />} onClick={() => remove(i)} />
                        </span>
                      )}
                    </div>
                    <TextArea className="mono" rows={Math.min(10, Math.max(2, s.command.split('\n').length + 1))} value={s.command} maxLength={8000} disabled={!editable} spellCheck={false} onChange={(e) => set(i, { command: e.target.value })} aria-label={t('docker.mig.step_command', { n: i + 1 })} />
                    {(warnings[i] ?? []).length > 0 && <span className="row row--tight">{warnings[i]!.map((w) => <Tag key={w} tone="danger">{tr('migwarn', w)}</Tag>)}</span>}
                  </div>
                ))}
                {editable && steps.length < 20 && <div><Button small icon={<Plus size={14} weight="bold" />} onClick={() => { setSteps([...steps, { title: '', command: '' }]); setWarnings([...warnings, []]); }}>{t('docker.mig.add')}</Button></div>}
              </section>

              {act.error && <Notice kind="error">{act.error}</Notice>}
              {!taskId && (
                <div className="row">
                  <Button variant="primary" icon={<Play size={16} weight="bold" />} disabled={!editable || !d.agent_ready || !d.docker_module || invalid || act.pending} onClick={() => { act.clearError(); setConfirm(true); }}>{t('docker.mig.run', { n: steps.length })}</Button>
                  <Button onClick={close}>{t('common.cancel')}</Button>
                </div>
              )}
              {taskId && (
                <section className="stack">
                  <h3 className="panel__title">{t('docker.mig.progress')} {task.data && <Tag tone={task.data.task.status === 'succeeded' ? 'ok' : running ? 'info' : 'danger'}>{tr('task', task.data.task.status)}</Tag>}</h3>
                  {task.data ? <p style={{ margin: 0 }}>{task.data.task.message}</p> : <Spinner />}
                  <pre className="logbox">{(task.data?.task.result?.log_tail ?? []).join('\n') || t('docker.mig.waiting')}</pre>
                  {running && <p className="muted" style={{ margin: 0 }}>{t('docker.mig.keep_open')}</p>}
                  {finished && <div className="row"><Button variant="primary" onClick={close}>{t('common.close')}</Button></div>}
                </section>
              )}
            </>
          )}
        </div>
      )}
    </Dialog>
    {/* beside the window, not inside it: closing a dialog inside another one would close both */}
    <ConfirmDialog danger message={confirm ? t('docker.mig.confirm', { n: steps.length, host: target?.hostname ?? '', source: source === 'ai' ? t('docker.mig.confirm.ai') : '' }) : null} onNo={() => setConfirm(false)} onYes={() => void run()} />
    </>
  );
}
