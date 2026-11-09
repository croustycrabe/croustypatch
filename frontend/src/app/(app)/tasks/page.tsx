'use client';
import Link from 'next/link';
import { useRouter, useSearchParams } from 'next/navigation';
import { Suspense, useState } from 'react';
import { Button, Chip, ConfirmDialog, Drawer, Empty, Notice, PageHeader, Spinner, StateIcon, Table, Tag, TaskStatusLabel } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { Task, TaskEvent } from '@/lib/types';

type Filter = 'open' | 'approval' | 'all';
const QUERY: Record<Filter, string> = { open: '?status=open', approval: '?status=pending_approval', all: '' };

interface AiOpinion { model: string; contextual_risk: string; rationale: string[]; confidence: number; by: string }

function AiReview({ taskId, existing }: { taskId: string; existing?: AiOpinion }) {
  const { t, tr } = useI18n();
  const act = useAction();
  const [op, setOp] = useState<AiOpinion | undefined>(existing);
  return (
    <div className="stack" style={{ gap: 8 }}>
      <div><Button small disabled={act.pending} onClick={() => void act.run(async () => setOp(await api<AiOpinion>('POST', `/tasks/${taskId}/ai-review`)))}>{act.pending && <span className="cs-spinner" aria-hidden="true" />}{t('ai.review')}</Button></div>
      {act.error && <Notice kind="error">{act.error}</Notice>}
      {op && (
        <div className="alert" role="status">
          <StateIcon tone="info" size={20} />
          <div className="alert__body">
            <b>{t('ai.review.title')}</b> · {op.model} · {t('tasks.contextual')} {tr('risk', op.contextual_risk)} · {Math.round(op.confidence * 100)}%
            <ul style={{ margin: '4px 0 0', paddingLeft: 18 }}>{op.rationale.map((r, i) => <li key={i}>{r}</li>)}</ul>
            <span className="sub">{t('ai.review.note')}</span>
          </div>
        </div>
      )}
    </div>
  );
}

function Detail({ id, onChanged }: { id: string; onChanged: () => void }) {
  const { t, tr, date } = useI18n();
  const { can } = useAuth();
  const d = useApi<{ task: Task; events: TaskEvent[] }>(`/tasks/${id}`, { poll: 4000 });
  const act = useAction();
  const [confirmRollback, setConfirmRollback] = useState(false);
  if (!d.data) return d.error ? <Notice kind="error">{d.error.message}</Notice> : <Spinner />;
  const { task, events } = d.data;
  const a = task.decision?.assessment;
  const post = (path: string) => act.run(async () => { await api('POST', `/tasks/${id}${path}`); d.reload(); onChanged(); });
  const canRollback = task.kind === 'patch' && ['succeeded', 'failed', 'soaking'].includes(task.status) && can('tasks:create');

  return (
    <div className="stack">
      {act.error && <Notice kind="error">{act.error}</Notice>}
      <dl className="kv">
        <dt>{t('tasks.col.host')}</dt><dd><Link href={`/host?id=${task.host_id}`}>{task.hostname}</Link></dd>
        <dt>{t('common.status')}</dt><dd><TaskStatusLabel status={task.status} />{task.message && <span className="muted"> · {task.message}</span>}</dd>
        <dt>{t('tasks.col.by')}</dt><dd>{task.requested_by}</dd>
        {task.approved_by && (<><dt>{t('tasks.approved_by')}</dt><dd>{task.approved_by}</dd></>)}
        <dt>{t('tasks.scheduled')}</dt><dd>{date(task.scheduled_for)}</dd>
        {task.soak_until && task.status === 'soaking' && (<><dt>{t('tasks.soak_until')}</dt><dd>{date(task.soak_until)}</dd></>)}
      </dl>

      <div className="row">
        {task.status === 'pending_approval' && can('tasks:write') && (
          <>
            <Button variant="primary" disabled={act.pending} onClick={() => void post('/approve')}>{t('tasks.approve')}</Button>
            <Button disabled={act.pending} onClick={() => void post('/reject')}>{t('tasks.reject')}</Button>
          </>
        )}
        {['pending_approval', 'queued'].includes(task.status) && can('tasks:write') && <Button disabled={act.pending} onClick={() => void post('/cancel')}>{t('tasks.cancel')}</Button>}
        {canRollback && <Button disabled={act.pending} onClick={() => setConfirmRollback(true)}>{t('tasks.rollback')}</Button>}
      </div>

      {a && (
        <section className="stack" aria-label={t('tasks.decision')}>
          <h3 className="panel__title">{t('tasks.decision')}</h3>
          <div className="row">
            <Tag>{task.decision?.policy_name}</Tag>
            <Tag>{t('tasks.contextual')} · {tr('risk', a.contextual_risk)}</Tag>
            <Tag>{t('tasks.stability')} · {tr('risk', a.stability_risk)}</Tag>
            <Tag>{tr('action', a.action)}</Tag>
            <Tag>{t('tasks.engine')} · {a.engine}</Tag>
          </div>
          <ul style={{ margin: 0, paddingLeft: 20, fontSize: 13 }}>{a.rationale.map((r, i) => <li key={i}>{r}</li>)}</ul>
          {task.kind === 'patch' && <AiReview taskId={task.id} existing={(task.decision as { ai_review?: AiOpinion } | null)?.ai_review} />}
        </section>
      )}

      {task.packages.length > 0 && (
        <section className="stack">
          <h3 className="panel__title">{t('tasks.col.packages')}</h3>
          <div className="pkgs">{task.packages.map((p) => <Tag key={p.name}>{p.name} {p.from ?? '∅'} → {p.to ?? '∅'}</Tag>)}</div>
        </section>
      )}
      {task.result?.reboot_required && <p className="label">{t('tasks.reboot')}</p>}

      <section className="stack">
        <h3 className="panel__title">{t('tasks.timeline')}</h3>
        <ol className="timeline">
          {events.map((e) => (
            <li key={e.id}><time>{date(e.ts, { timeStyle: 'medium' })}</time><span><b>{tr('task', e.status)}</b> · {e.message}</span></li>
          ))}
        </ol>
      </section>

      {task.result?.log_tail && task.result.log_tail.length > 0 && (
        <section className="stack">
          <h3 className="panel__title">{t('tasks.log')}</h3>
          <pre className="logbox">{task.result.log_tail.join('\n')}</pre>
        </section>
      )}
      <ConfirmDialog message={confirmRollback ? t('tasks.confirm_rollback') : null} onNo={() => setConfirmRollback(false)} onYes={() => { setConfirmRollback(false); void post('/rollback'); }} />
    </div>
  );
}

function Tasks() {
  const { t, tr, ago } = useI18n();
  const router = useRouter();
  const params = useSearchParams();
  const open = params.get('task');
  const initial = params.get('filter');
  const [filter, setFilter] = useState<Filter>(initial === 'approval' || initial === 'all' ? initial : 'open');
  const tasks = useApi<Task[]>(`/tasks${QUERY[filter]}`, { poll: 6000 });

  return (
    <>
      <PageHeader title={t('tasks.title')} subtitle={t('tasks.subtitle')} />
      <div className="stack">
        <div className="chips" role="group" aria-label={t('common.status')} style={{ marginBottom: 14 }}>
          {(['open', 'approval', 'all'] as const).map((f) => <Chip key={f} active={filter === f} onClick={() => setFilter(f)}>{t(`tasks.filter.${f}`)}</Chip>)}
        </div>
        {tasks.error && <Notice kind="error">{tasks.error.message}</Notice>}
        {!tasks.data ? <Spinner /> : (
          <Table head={[t('tasks.col.task'), t('tasks.col.host'), t('common.status'), t('tasks.col.packages'), t('tasks.col.by'), t('tasks.col.when')]} empty={tasks.data.length === 0 && <Empty>{t('tasks.none')}</Empty>}>
            {tasks.data.map((k) => (
              <tr key={k.id}>
                <td><Link href={`/tasks?task=${k.id}`}>{tr('kind', k.kind)}</Link></td>
                <td><Link href={`/host?id=${k.host_id}`}>{k.hostname}</Link></td>
                <td><TaskStatusLabel status={k.status} /></td>
                <td className="wrap">{k.packages.length ? <div className="pkgs">{k.packages.slice(0, 4).map((p) => <Tag key={p.name}>{p.name}</Tag>)}{k.packages.length > 4 && <Tag>+{k.packages.length - 4}</Tag>}</div> : '—'}</td>
                <td className="wrap">{k.requested_by}</td>
                <td className="nowrap">{ago(k.updated_at)}</td>
              </tr>
            ))}
          </Table>
        )}
      </div>
      <Drawer open={!!open} title={t('tasks.detail')} onClose={() => router.replace('/tasks/')}>
        {open && <Detail id={open} onChanged={() => void tasks.reload()} />}
      </Drawer>
    </>
  );
}

export default function Page() {
  return (
    <>
      <Suspense fallback={<Spinner />}>
        <Tasks />
      </Suspense>
    </>
  );
}
