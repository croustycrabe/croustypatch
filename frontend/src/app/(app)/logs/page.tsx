'use client';
import Link from 'next/link';
import { useCallback, useEffect, useState } from 'react';
import { Button, Chip, Empty, Field, Notice, PageHeader, Select, Spinner, Switch, Table, Tag, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useI18n } from '@/lib/i18n';
import type { LogEntry } from '@/lib/types';

const PAGE = 100;
const SOURCES = ['', 'tasks', 'notifications', 'reports', 'server'] as const;
const TONE = { error: 'danger', warn: 'warn', info: 'neutral' } as const;

/** Administration > Logs: what the server did (tasks, notifications, reports, its own messages), newest first. */
function Logs() {
  const { t, date } = useI18n();
  const [source, setSource] = useState<(typeof SOURCES)[number]>('');
  const [level, setLevel] = useState('');
  const [q, setQ] = useState('');
  const [live, setLive] = useState(true);
  const [rows, setRows] = useState<LogEntry[] | null>(null);
  const [more, setMore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (before?: string) => {
    const qs = new URLSearchParams({ limit: String(PAGE) });
    if (source) qs.set('source', source);
    if (level) qs.set('level', level);
    if (q.trim()) qs.set('q', q.trim());
    if (before) qs.set('before', before);
    try {
      const page = await api<LogEntry[]>('GET', `/logs?${qs}`);
      setRows((prev) => (before && prev ? [...prev, ...page] : page));
      setMore(page.length === PAGE);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [source, level, q]);

  useEffect(() => {
    const id = setTimeout(() => void load(), 250);
    return () => clearTimeout(id);
  }, [load]);
  // follow new lines while only the first page is shown
  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => { if ((rows?.length ?? 0) <= PAGE) void load(); }, 10000);
    return () => clearInterval(id);
  }, [live, load, rows?.length]);

  return (
    <>
      <PageHeader title={t('logs.title')} subtitle={t('logs.subtitle')} />
      <div className="stack">
        {error && <Notice kind="error">{error}</Notice>}
        <div className="row">
          {SOURCES.map((s) => <Chip key={s || 'all'} active={source === s} onClick={() => setSource(s)}>{t(`logs.src.${s || 'all'}` as 'logs.src.all')}</Chip>)}
        </div>
        <div className="form-grid">
          <Field label={t('logs.level')}>{(i) => (
            <Select id={i} value={level} onChange={(e) => setLevel(e.target.value)}>
              <option value="">{t('logs.level.all')}</option><option value="warn">{t('logs.level.warn')}</option><option value="error">{t('logs.level.error')}</option>
            </Select>)}</Field>
          <Field label={t('logs.search')}>{(i) => <TextInput id={i} type="search" value={q} placeholder="docker-data failed" onChange={(e) => setQ(e.target.value)} />}</Field>
        </div>
        <div className="row" style={{ justifyContent: 'space-between' }}>
          <span className="muted">{t('logs.server.note')}</span>
          <Switch checked={live} onChange={setLive}>{t('logs.live')}</Switch>
        </div>
        {!rows ? <Spinner /> : (
          <Table head={[t('audit.col.when'), t('logs.col.level'), t('logs.col.source'), t('logs.col.what'), t('logs.col.message')]} empty={rows.length === 0 && <Empty>{t('logs.none')}</Empty>}>
            {rows.map((r, i) => (
              <tr key={`${r.ts}-${r.source}-${r.ref_id ?? i}-${i}`}>
                <td className="nowrap">{date(r.ts, { dateStyle: 'short', timeStyle: 'medium' })}</td>
                <td><Tag tone={TONE[r.level]}>{t(`logs.level.${r.level}` as 'logs.level.warn')}</Tag></td>
                <td>{t(`logs.src.${r.source}` as 'logs.src.all')}</td>
                <td className="wrap">
                  {r.source === 'tasks' && r.ref_id ? <Link href={`/tasks?task=${r.ref_id}`}>{r.title}</Link> : <span className={r.source === 'server' ? 'mono' : undefined}>{r.title}</span>}
                  {r.hostname && <span className="sub">{r.host_id ? <Link href={`/host?id=${r.host_id}`}>{r.hostname}</Link> : r.hostname}</span>}
                </td>
                <td className="wrap">{r.message}</td>
              </tr>
            ))}
          </Table>
        )}
        {more && rows && <div><Button onClick={() => void load(rows[rows.length - 1]?.ts)}>{t('common.more')}</Button></div>}
      </div>
    </>
  );
}

export default function Page() {
  return <Logs />;
}
