'use client';
import { Copy, Key, Plus } from '@phosphor-icons/react';
import { useState } from 'react';
import { Button, Card, ConfirmDialog, Dialog, Empty, Field, Notice, PageHeader, Spinner, Table, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';

interface Token { id: string; name: string; prefix: string; expires_at: string | null; last_used_at: string | null; created_at: string }

export default function Page() {
  const { t, date, ago } = useI18n();
  const toast = useToast();
  const tokens = useApi<Token[]>('/auth/tokens');
  const act = useAction();
  const [form, setForm] = useState<{ name: string; days: number } | null>(null);
  const [created, setCreated] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<Token | null>(null);
  const mcpUrl = typeof window === 'undefined' ? '/api/v1/mcp' : `${window.location.origin}/api/v1/mcp`;
  const sample = JSON.stringify({ mcpServers: { croustypatch: { type: 'http', url: mcpUrl, headers: { Authorization: 'Bearer cpat_…' } } } }, null, 2);
  const copy = (s: string) => { void navigator.clipboard?.writeText(s); toast.success('OK'); };

  const create = () => act.run(async () => {
    const r = await api<{ token: string }>('POST', '/auth/tokens', form);
    setForm(null);
    setCreated(r.token);
    tokens.reload();
  });

  return (
    <>
      <PageHeader title={t('account.title')} subtitle={t('account.subtitle')} />
      <div className="stack stack--lg">
        <Card title={t('account.tokens')} actions={<Button variant="primary" icon={<Plus size={16} weight="bold" />} onClick={() => { act.clearError(); setForm({ name: '', days: 90 }); }}>{t('account.token.new')}</Button>}>
          {created && (
            <div className="stack" style={{ marginBottom: 14 }}>
              <Notice kind="ok">{t('account.token.created')}</Notice>
              <div className="row"><code className="cs-inline-code" style={{ wordBreak: 'break-all' }}>{created}</code><Button small icon={<Copy size={16} weight="bold" />} aria-label="Copy" onClick={() => copy(created)} /></div>
            </div>
          )}
          {act.error && !form && <Notice kind="error">{act.error}</Notice>}
          {!tokens.data ? <Spinner /> : (
            <Table head={[t('common.name'), t('account.token.prefix'), t('account.token.expires'), t('account.token.used'), t('common.actions')]} empty={tokens.data.length === 0 && <Empty icon={<Key size={22} weight="bold" />}>{t('account.token.none')}</Empty>}>
              {tokens.data.map((k) => (
                <tr key={k.id}>
                  <td className="strong">{k.name}</td>
                  <td className="mono">{k.prefix}…</td>
                  <td className="nowrap">{date(k.expires_at, { dateStyle: 'medium' })}</td>
                  <td className="nowrap">{k.last_used_at ? ago(k.last_used_at) : '—'}</td>
                  <td><Button small onClick={() => setRevoking(k)}>{t('account.token.revoke')}</Button></td>
                </tr>
              ))}
            </Table>
          )}
        </Card>
        <Card title={t('account.mcp')}>
          <div className="stack">
            <p>{t('account.mcp.text')}</p>
            <div className="cs-code">
              <div className="cs-code__bar"><span>mcp.json</span><Button small variant="ghost" icon={<Copy size={16} weight="bold" />} aria-label="Copy" onClick={() => copy(sample)} /></div>
              <pre>{sample}</pre>
            </div>
          </div>
        </Card>
      </div>
      <Dialog open={!!form} title={t('account.token.new')} onClose={() => setForm(null)}>
        {form && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void create(); }}>
            <Field label={t('account.token.name')}>{(i) => <TextInput id={i} value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="Claude Desktop" required autoFocus />}</Field>
            <Field label={t('account.token.days')}>{(i) => <TextInput id={i} type="number" min={1} max={365} value={form.days} onChange={(e) => setForm({ ...form, days: Number(e.target.value) })} />}</Field>
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row"><Button variant="primary" type="submit" disabled={act.pending}>{t('account.token.new')}</Button><Button onClick={() => setForm(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>
      <ConfirmDialog danger message={revoking ? t('account.token.confirm_revoke') : null} onNo={() => setRevoking(null)} onYes={() => { const k = revoking!; setRevoking(null); void act.run(async () => { await api('DELETE', `/auth/tokens/${k.id}`); tokens.reload(); }); }} />
    </>
  );
}
