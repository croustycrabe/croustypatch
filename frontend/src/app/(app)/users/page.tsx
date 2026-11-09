'use client';
import { useState } from 'react';
import { Button, Check, ConfirmDialog, Dialog, Empty, Field, Notice, PageHeader, Select, Spinner, Table, Tabs, Tag, TextInput } from '@/components/ui';
import { allows, api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { Role, UserRow } from '@/lib/types';

const MODULES = ['dashboard', 'hosts', 'groups', 'policies', 'tasks', 'vulnerabilities', 'waivers', 'enrollment', 'audit', 'settings', 'users', 'roles'] as const;
const ACTIONS = ['read', 'create', 'write', 'delete'] as const;
const EXTRA = ['waivers:approve_ciso', 'waivers:approve_business'] as const;

/** Expand wildcards into the explicit grid so the editor shows what a role really grants. */
function explicit(perms: string[]): Set<string> {
  const out = new Set<string>();
  for (const m of MODULES) for (const a of ACTIONS) if (allows(perms, `${m}:${a}`)) out.add(`${m}:${a}`);
  for (const e of EXTRA) if (allows(perms, e)) out.add(e);
  return out;
}

function PermissionGrid({ value, onChange, readOnly, mine }: { value: Set<string>; onChange: (s: Set<string>) => void; readOnly: boolean; mine: string[] }) {
  const { t, tr } = useI18n();
  const toggle = (k: string, on: boolean) => { const n = new Set(value); if (on) n.add(k); else n.delete(k); onChange(n); };
  // You can only hand out rights you hold yourself (the server enforces this too).
  const disabled = (k: string) => readOnly || !allows(mine, k);
  return (
    <div className="perm-grid" role="group" aria-label={t('roles.permissions')}>
      <span className="h">{t('roles.module')}</span>
      {ACTIONS.map((a) => <span key={a} className="h">{tr('perm', a)}</span>)}
      {MODULES.map((m) => (
        <div key={m} style={{ display: 'contents' }}>
          <span>{m}</span>
          {ACTIONS.map((a) => {
            const k = `${m}:${a}`;
            return <input key={a} type="checkbox" aria-label={k} checked={value.has(k)} disabled={disabled(k)} onChange={(e) => toggle(k, e.target.checked)} style={{ accentColor: 'var(--primary)', width: 18, height: 18 }} />;
          })}
        </div>
      ))}
      <div style={{ gridColumn: '1 / -1', display: 'flex', gap: 'var(--space-6)', flexWrap: 'wrap', paddingTop: 'var(--space-2)' }}>
        {EXTRA.map((k) => <Check key={k} checked={value.has(k)} disabled={disabled(k)} onChange={(v) => toggle(k, v)}>{k}</Check>)}
      </div>
    </div>
  );
}

function Users() {
  const { t, date } = useI18n();
  const { can, me } = useAuth();
  const [tab, setTab] = useState<'users' | 'roles'>('users');
  const users = useApi<UserRow[]>(can('users:read') ? '/users' : null);
  const roles = useApi<Role[]>(can('roles:read') ? '/roles' : null);
  const act = useAction();

  const [uForm, setUForm] = useState<{ id?: string; username: string; display_name: string; email: string; password: string; role_id: string } | null>(null);
  const [reset, setReset] = useState<{ id: string; username: string; password: string } | null>(null);
  const [delUser, setDelUser] = useState<UserRow | null>(null);
  const [rForm, setRForm] = useState<{ id?: string; name: string; description: string; perms: Set<string>; builtin: boolean } | null>(null);
  const [delRole, setDelRole] = useState<Role | null>(null);

  const roleName = (id: string) => roles.data?.find((r) => r.id === id)?.name ?? '—';
  const saveUser = () => act.run(async () => {
    const f = uForm!;
    if (f.id) await api('PATCH', `/users/${f.id}`, { display_name: f.display_name, email: f.email || null, role_id: f.role_id });
    else await api('POST', '/users', { username: f.username, display_name: f.display_name, email: f.email || null, password: f.password, role_id: f.role_id });
    setUForm(null);
    users.reload();
  });
  const saveRole = () => act.run(async () => {
    const f = rForm!;
    const body = { name: f.name, description: f.description, permissions: [...f.perms] };
    if (f.id) await api('PUT', `/roles/${f.id}`, body);
    else await api('POST', '/roles', body);
    setRForm(null);
    roles.reload();
  });

  return (
    <>
      <PageHeader title={t('users.title')} subtitle={t('users.subtitle')}
        actions={tab === 'users' ? can('users:create') && <Button variant="primary" onClick={() => { act.clearError(); setUForm({ username: '', display_name: '', email: '', password: '', role_id: roles.data?.find((r) => r.name === 'operator')?.id ?? '' }); }}>{t('users.new')}</Button>
          : can('roles:create') && <Button variant="primary" onClick={() => { act.clearError(); setRForm({ name: '', description: '', perms: new Set(['hosts:read']), builtin: false }); }}>{t('roles.new')}</Button>} />
      <Tabs value={tab} onChange={setTab} tabs={[...(can('users:read') ? [{ id: 'users' as const, label: t('users.tab.users') }] : []), ...(can('roles:read') ? [{ id: 'roles' as const, label: t('users.tab.roles') }] : [])]} />
      {act.error && !uForm && !rForm && !reset && <Notice kind="error">{act.error}</Notice>}

      {tab === 'users' && (!users.data ? (users.error ? <Notice kind="error">{users.error.message}</Notice> : <Spinner />) : (
        <Table head={[t('users.username'), t('users.role'), t('users.last_login'), t('common.status'), t('common.actions')]} empty={users.data.length === 0 && <Empty>{t('common.empty')}</Empty>}>
          {users.data.map((u) => (
            <tr key={u.id}>
              <td>{u.username}{u.is_fallback_admin && <> <Tag>{t('users.fallback')}</Tag></>}<span className="sub">{u.display_name || u.email || u.auth_source}</span></td>
              <td>{roleName(u.role_id)}</td>
              <td className="nowrap">{u.last_login_at ? date(u.last_login_at) : t('common.never')}</td>
              <td>{u.disabled ? <Tag>{t('users.disabled')}</Tag> : '—'}</td>
              <td><div className="row row--tight">
                {can('users:write') && <Button small onClick={() => { act.clearError(); setUForm({ id: u.id, username: u.username, display_name: u.display_name, email: u.email ?? '', password: '', role_id: u.role_id }); }}>{t('common.edit')}</Button>}
                {can('users:write') && u.id !== me?.id && <Button small onClick={() => void act.run(async () => { await api('PATCH', `/users/${u.id}`, { disabled: !u.disabled }); users.reload(); })}>{u.disabled ? t('users.enable') : t('users.disable')}</Button>}
                {can('users:write') && u.auth_source === 'local' && <Button small onClick={() => { act.clearError(); setReset({ id: u.id, username: u.username, password: '' }); }}>{t('users.reset')}</Button>}
                {can('users:delete') && u.id !== me?.id && <Button small onClick={() => setDelUser(u)}>{t('common.delete')}</Button>}
              </div></td>
            </tr>
          ))}
        </Table>
      ))}

      {tab === 'roles' && (!roles.data ? (roles.error ? <Notice kind="error">{roles.error.message}</Notice> : <Spinner />) : (
        <Table head={[t('common.name'), t('common.description'), t('roles.permissions'), t('common.actions')]}>
          {roles.data.map((r) => (
            <tr key={r.id}>
              <td>{r.name}{r.builtin && <> <Tag>{t('roles.builtin')}</Tag></>}</td>
              <td className="wrap">{r.description || '—'}</td>
              <td className="wrap"><div className="pkgs">{r.permissions.slice(0, 6).map((p) => <Tag key={p}>{p}</Tag>)}{r.permissions.length > 6 && <Tag>+{r.permissions.length - 6}</Tag>}</div></td>
              <td><div className="row row--tight">
                <Button small onClick={() => { act.clearError(); setRForm({ id: r.id, name: r.name, description: r.description, perms: explicit(r.permissions), builtin: r.builtin }); }}>{r.builtin || !can('roles:write') ? t('common.details') : t('common.edit')}</Button>
                {!r.builtin && can('roles:delete') && <Button small onClick={() => setDelRole(r)}>{t('common.delete')}</Button>}
              </div></td>
            </tr>
          ))}
        </Table>
      ))}

      <Dialog open={!!uForm} title={uForm?.id ? t('common.edit') : t('users.new')} onClose={() => setUForm(null)}>
        {uForm && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void saveUser(); }}>
            <Field label={t('users.username')}>{(i) => <TextInput id={i} value={uForm.username} disabled={!!uForm.id} onChange={(e) => setUForm({ ...uForm, username: e.target.value })} required autoFocus />}</Field>
            <Field label={t('users.display')}>{(i) => <TextInput id={i} value={uForm.display_name} onChange={(e) => setUForm({ ...uForm, display_name: e.target.value })} />}</Field>
            <Field label={t('users.email')}>{(i) => <TextInput id={i} type="email" value={uForm.email} onChange={(e) => setUForm({ ...uForm, email: e.target.value })} />}</Field>
            {!uForm.id && <Field label={t('users.password')} hint={t('users.password.hint')}>{(i) => <TextInput id={i} type="password" autoComplete="new-password" minLength={12} value={uForm.password} onChange={(e) => setUForm({ ...uForm, password: e.target.value })} required />}</Field>}
            <Field label={t('users.role')}>{(i) => <Select id={i} value={uForm.role_id} onChange={(e) => setUForm({ ...uForm, role_id: e.target.value })} required><option value="" />{roles.data?.map((r) => <option key={r.id} value={r.id}>{r.name}</option>)}</Select>}</Field>
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row"><Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button><Button onClick={() => setUForm(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>

      <Dialog open={!!reset} title={t('users.reset')} onClose={() => setReset(null)}>
        {reset && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void act.run(async () => { await api('PATCH', `/users/${reset.id}`, { password: reset.password }); setReset(null); }); }}>
            <p>{reset.username}</p>
            <Field label={t('users.password')} hint={t('users.password.hint')}>{(i) => <TextInput id={i} type="password" autoComplete="new-password" minLength={12} value={reset.password} onChange={(e) => setReset({ ...reset, password: e.target.value })} required autoFocus />}</Field>
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row"><Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button><Button onClick={() => setReset(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>

      <Dialog open={!!rForm} wide title={rForm?.name || t('roles.new')} onClose={() => setRForm(null)}>
        {rForm && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); void saveRole(); }}>
            {rForm.builtin && <Notice>{t('roles.readonly')}</Notice>}
            <div className="form-grid">
              <Field label={t('common.name')}>{(i) => <TextInput id={i} value={rForm.name} disabled={rForm.builtin} onChange={(e) => setRForm({ ...rForm, name: e.target.value })} required />}</Field>
              <Field label={t('common.description')}>{(i) => <TextInput id={i} value={rForm.description} disabled={rForm.builtin} onChange={(e) => setRForm({ ...rForm, description: e.target.value })} />}</Field>
            </div>
            <PermissionGrid value={rForm.perms} readOnly={rForm.builtin || !can(rForm.id ? 'roles:write' : 'roles:create')} mine={me?.permissions ?? []} onChange={(perms) => setRForm({ ...rForm, perms })} />
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="row">{!rForm.builtin && <Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button>}<Button onClick={() => setRForm(null)}>{t('common.close')}</Button></div>
          </form>
        )}
      </Dialog>

      <ConfirmDialog danger message={delUser ? t('users.confirm_delete') : null} onNo={() => setDelUser(null)} onYes={() => { const u = delUser!; setDelUser(null); void act.run(async () => { await api('DELETE', `/users/${u.id}`); users.reload(); }); }} />
      <ConfirmDialog danger message={delRole ? t('roles.confirm_delete') : null} onNo={() => setDelRole(null)} onYes={() => { const r = delRole!; setDelRole(null); void act.run(async () => { await api('DELETE', `/roles/${r.id}`); roles.reload(); }); }} />
    </>
  );
}

export default function Page() {
  return <><Users /></>;
}
