'use client';
import { useState } from 'react';
import { Button, Dialog, Empty, Field, Notice, PageHeader, Spinner, Table, Tag, TextArea, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { Vulnerability } from '@/lib/types';

function Vulns() {
  const { t, date } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const [q, setQ] = useState('');
  const list = useApi<Vulnerability[]>(`/vulnerabilities?q=${encodeURIComponent(q)}`);
  const [importing, setImporting] = useState(false);
  const [text, setText] = useState('');
  const act = useAction();

  const run = () => act.run(async () => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      throw new Error(t('pol.invalid_json'));
    }
    const r = await api<{ imported: number }>('PUT', '/vulnerabilities', parsed);
    toast.success(t('vuln.import.done', { n: r.imported }));
    setImporting(false);
    setText('');
    list.reload();
  });

  return (
    <>
      <PageHeader title={t('vuln.title')} subtitle={t('vuln.subtitle')}
        actions={can('vulnerabilities:write') && <Button onClick={() => { act.clearError(); setImporting(true); }}>{t('vuln.import')}</Button>} />
      <div className="stack">
        <TextInput type="search" aria-label={t('vuln.search')} placeholder={t('vuln.search')} value={q} onChange={(e) => setQ(e.target.value)} style={{ maxWidth: '24rem' }} />
        {list.error && <Notice kind="error">{list.error.message}</Notice>}
        {!list.data ? <Spinner /> : (
          <Table head={[t('vuln.col.cve'), t('vuln.col.cvss'), t('vuln.col.summary'), t('vuln.col.exposed')]} empty={list.data.length === 0 && <Empty>{t('vuln.none')}</Empty>}>
            {list.data.map(({ vulnerability: v, exposed_hosts }) => (
              <tr key={v.cve_id}>
                <td className="nowrap">{v.cve_id}{v.exploited && <> <Tag>{t('vuln.exploited')}</Tag></>}<span className="sub">{v.published_at ? date(v.published_at, { dateStyle: 'medium' }) : v.source}</span></td>
                <td className="nowrap">{v.cvss ?? '—'}</td>
                <td className="wrap">{v.summary || '—'}</td>
                <td className="num"><span className={exposed_hosts > 0 ? 'label' : ''}>{exposed_hosts}</span></td>
              </tr>
            ))}
          </Table>
        )}
      </div>
      <Dialog open={importing} wide title={t('vuln.import')} onClose={() => setImporting(false)}>
        <form className="stack" onSubmit={(e) => { e.preventDefault(); void run(); }}>
          <Field label="JSON" hint={t('vuln.import.hint')}>{(i) => <TextArea id={i} className="input--code" value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} required />}</Field>
          {act.error && <Notice kind="error">{act.error}</Notice>}
          <div className="row"><Button variant="primary" type="submit" disabled={act.pending || !text.trim()}>{t('vuln.import.run')}</Button><Button onClick={() => setImporting(false)}>{t('common.cancel')}</Button></div>
        </form>
      </Dialog>
    </>
  );
}

export default function Page() {
  return <><Vulns /></>;
}
