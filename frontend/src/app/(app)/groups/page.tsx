'use client';
import { useState } from 'react';
import { Button, ColorPicker, ConfirmDialog, Dialog, Empty, Field, GroupTag, Notice, PageHeader, Spinner, Table, TextInput } from '@/components/ui';
import { Palette } from '@phosphor-icons/react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { HostGroup } from '@/lib/types';
import { useToast } from '@/lib/toast';
import Link from 'next/link';

function Groups() {
  const { t } = useI18n();
  const { can } = useAuth();
  const groups = useApi<HostGroup[]>('/groups');
  const [editing, setEditing] = useState<Partial<HostGroup> | null>(null);
  const [deleting, setDeleting] = useState<HostGroup | null>(null);
  const act = useAction();
  const toast = useToast();
  const [picking, setPicking] = useState<{ id: string; x: number; y: number } | null>(null);

  const save = () => act.run(async () => {
    const g = editing!;
    if (g.id) await api('PUT', `/groups/${g.id}`, { name: g.name, description: g.description ?? '', color: g.color ?? '' });
    else await api('POST', '/groups', { name: g.name, description: g.description ?? '', color: g.color ?? '' });
    setEditing(null);
    groups.reload();
    toast.success(t('common.saved'));
  });

  return (
    <>
      <PageHeader title={t('groups.title')} subtitle={t('groups.subtitle')}
        actions={can('groups:create') && <Button variant="primary" onClick={() => { act.clearError(); setEditing({ name: '', description: '', color: '' }); }}>{t('groups.new')}</Button>} />
      {groups.error && <Notice kind="error">{groups.error.message}</Notice>}
      {!groups.data ? <Spinner /> : (
        <Table head={[t('common.name'), t('common.description'), t('groups.hosts'), t('common.actions')]} empty={groups.data.length === 0 && <Empty>{t('groups.none')}</Empty>}>
          {groups.data.map((g) => (
            <tr key={g.id}>
              <td>
                <div className="row row--tight">
                  <Link href={`/hosts?group=${g.id}`}><GroupTag group={g} /></Link>
                  {can('groups:write') && (
                    <Button small variant="ghost" icon={<Palette size={16} />} aria-label={t('groups.color.change')} title={t('groups.color.change')} aria-expanded={picking?.id === g.id}
                      onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setPicking(picking?.id === g.id ? null : { id: g.id, x: Math.min(r.left, window.innerWidth - 260), y: r.bottom + 6 }); }} />
                  )}
                </div>
              </td>
              <td className="wrap">{g.description || '—'}</td>
              <td>{g.host_count ?? 0}</td>
              <td>
                <div className="row row--tight">
                  {can('groups:write') && <Button small onClick={() => { act.clearError(); setEditing(g); }}>{t('common.edit')}</Button>}
                  {can('groups:delete') && <Button small onClick={() => setDeleting(g)}>{t('common.delete')}</Button>}
                </div>
              </td>
            </tr>
          ))}
        </Table>
      )}
      {picking && (() => {
        const g = groups.data?.find((x) => x.id === picking.id);
        return g ? (
          <>
            <div className="clickaway" onClick={() => setPicking(null)} />
            <div className="popover popover--fixed" style={{ left: picking.x, top: Math.min(picking.y, window.innerHeight - 130) }} onKeyDown={(e) => e.key === 'Escape' && setPicking(null)}>
              <ColorPicker name={g.name} value={g.color ?? ''} onChange={(c) => { setPicking(null); void act.run(async () => { await api('PUT', `/groups/${g.id}`, { name: g.name, description: g.description, color: c }); groups.reload(); toast.success(t('common.saved')); }); }} />
            </div>
          </>
        ) : null;
      })()}
      <Dialog open={!!editing} title={editing?.id ? t('common.edit') : t('groups.new')} onClose={() => setEditing(null)}>
        <form className="stack" onSubmit={(e) => { e.preventDefault(); void save(); }}>
          <Field label={t('common.name')}>{(i) => <TextInput id={i} value={editing?.name ?? ''} onChange={(e) => setEditing({ ...editing, name: e.target.value })} required autoFocus />}</Field>
          <Field label={t('common.description')}>{(i) => <TextInput id={i} value={editing?.description ?? ''} onChange={(e) => setEditing({ ...editing, description: e.target.value })} />}</Field>
          <Field label={t('groups.color')}>{() => <ColorPicker live name={editing?.name ?? ''} value={editing?.color ?? ''} onChange={(c) => setEditing({ ...editing, color: c })} />}</Field>
          {editing?.name && <div><GroupTag group={{ name: editing.name, color: editing.color }} /></div>}
          {act.error && <Notice kind="error">{act.error}</Notice>}
          <div className="row"><Button variant="primary" type="submit" disabled={act.pending}>{t('common.save')}</Button><Button onClick={() => setEditing(null)}>{t('common.cancel')}</Button></div>
        </form>
      </Dialog>
      <ConfirmDialog danger message={deleting ? t('groups.confirm_delete') : null} onNo={() => setDeleting(null)} onYes={() => { const g = deleting!; setDeleting(null); void act.run(async () => { await api('DELETE', `/groups/${g.id}`); groups.reload(); }); }} />
      {act.error && !editing && <Notice kind="error">{act.error}</Notice>}
    </>
  );
}

export default function Page() {
  return <><Groups /></>;
}
