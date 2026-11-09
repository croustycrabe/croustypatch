'use client';
import { useCallback, useEffect, useState } from 'react';
import { Button, Empty, Field, Notice, PageHeader, Spinner, Table, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAction } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { AuditEntry, AuditVerification } from '@/lib/types';

const PAGE = 50;

function Audit() {
  const { t, date } = useI18n();
  const [actor, setActor] = useState('');
  const [action, setAction] = useState('');
  const [rows, setRows] = useState<AuditEntry[] | null>(null);
  const [more, setMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [verdict, setVerdict] = useState<AuditVerification | null>(null);
  const act = useAction();

  const load = useCallback(async (before?: number) => {
    const qs = new URLSearchParams({ limit: String(PAGE) });
    if (actor.trim()) qs.set('actor', actor.trim());
    if (action.trim()) qs.set('action', action.trim());
    if (before) qs.set('before', String(before));
    try {
      const page = await api<AuditEntry[]>('GET', `/audit?${qs}`);
      setRows((prev) => (before && prev ? [...prev, ...page] : page));
      setMore(page.length === PAGE);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [actor, action]);

  // Debounce the filters so typing does not hammer the server.
  useEffect(() => {
    const id = setTimeout(() => void load(), 250);
    return () => clearTimeout(id);
  }, [load]);

  const verify = () => act.run(async () => setVerdict(await api<AuditVerification>('GET', '/audit/verify')));

  return (
    <>
      <PageHeader title={t('audit.title')} subtitle={t('audit.subtitle')}
        actions={<Button variant="primary" disabled={act.pending} onClick={() => void verify()}>{t('audit.verify')}</Button>} />
      <div className="stack">
        {verdict && <Notice kind={verdict.ok ? 'ok' : 'error'}>{verdict.ok ? t('audit.verify.ok', { n: verdict.entries }) : t('audit.verify.bad', { seq: verdict.first_bad_seq ?? '?' })}</Notice>}
        {(error || act.error) && <Notice kind="error">{error ?? act.error}</Notice>}
        <div className="form-grid">
          <Field label={t('audit.filter.actor')}>{(i) => <TextInput id={i} value={actor} onChange={(e) => setActor(e.target.value)} />}</Field>
          <Field label={t('audit.filter.action')}>{(i) => <TextInput id={i} value={action} placeholder="task." onChange={(e) => setAction(e.target.value)} />}</Field>
        </div>
        {!rows ? <Spinner /> : (
          <Table head={[t('audit.col.seq'), t('audit.col.when'), t('audit.col.actor'), t('audit.col.action'), t('audit.col.target')]} empty={rows.length === 0 && <Empty>{t('audit.none')}</Empty>}>
            {rows.map((r) => (
              <tr key={r.seq}>
                <td className="num">{r.seq}</td>
                <td className="nowrap">{date(r.ts, { dateStyle: 'short', timeStyle: 'medium' })}</td>
                <td>{r.actor}{r.ip && <span className="sub">{r.ip}</span>}</td>
                <td className="wrap">{r.action}</td>
                <td className="wrap">{r.target_type ? `${r.target_type} ${r.target_id.slice(0, 8)}` : '—'}
                  {Object.keys(r.details).length > 0 && <span className="sub">{JSON.stringify(r.details).slice(0, 120)}</span>}</td>
              </tr>
            ))}
          </Table>
        )}
        {more && rows && <div><Button onClick={() => void load(rows[rows.length - 1]?.seq)}>{t('common.more')}</Button></div>}
      </div>
    </>
  );
}

export default function Page() {
  return <><Audit /></>;
}
