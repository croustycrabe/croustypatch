'use client';
import { Key } from '@phosphor-icons/react';
import { AddAgentButton } from '@/components/AddAgent';
import { Button, Empty, Notice, PageHeader, Spinner, Table, Tag } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { EnrollmentToken, HostGroup } from '@/lib/types';

function Enrollment() {
  const { t, date } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const list = useApi<EnrollmentToken[]>('/enrollment-tokens', { poll: 15000 });
  const groups = useApi<HostGroup[]>(can('groups:read') ? '/groups' : null);
  const act = useAction();

  const state = (k: EnrollmentToken) => (k.revoked ? t('enroll.revoked') : new Date(k.expires_at) < new Date() || k.used_count >= k.max_uses ? t('enroll.expired') : t('enroll.active'));
  const groupName = (id: string | null) => groups.data?.find((g) => g.id === id)?.name;

  return (
    <>
      <PageHeader title={t('enroll.title')} subtitle={t('enroll.subtitle')} actions={<AddAgentButton />} />
      {(act.error || list.error) && <Notice kind="error">{act.error ?? list.error?.message}</Notice>}
      {!list.data ? <Spinner /> : (
        <Table head={[t('agent.label'), t('agent.group'), t('agent.mode'), t('enroll.col.uses'), t('enroll.col.expires'), t('common.status'), '']} empty={list.data.length === 0 && <Empty icon={<Key size={22} weight="bold" />} action={<AddAgentButton />}>{t('enroll.none')}</Empty>}>
          {list.data.map((k) => (
            <tr key={k.id}>
              <td className="wrap">{k.label || '—'}<span className="sub">{k.created_by}</span></td>
              <td>{groupName(k.group_id) ?? '—'}</td>
              <td>{k.comm_mode} · {k.environment}</td>
              <td>{k.used_count} / {k.max_uses}</td>
              <td className="nowrap">{date(k.expires_at)}</td>
              <td><Tag tone={state(k) === t('enroll.active') ? undefined : 'brick'}>{state(k)}</Tag></td>
              <td>{!k.revoked && can('enrollment:delete') && <Button small variant="danger" disabled={act.pending} onClick={() => void act.run(async () => { await api('DELETE', `/enrollment-tokens/${k.id}`); toast.success(t('enroll.revoked')); list.reload(); })}>{t('enroll.revoke')}</Button>}</td>
            </tr>
          ))}
        </Table>
      )}
    </>
  );
}

export default function Page() {
  return <><Enrollment /></>;
}
