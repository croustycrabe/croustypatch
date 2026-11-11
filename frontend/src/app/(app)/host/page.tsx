'use client';
import { ArrowCircleUp, ArrowsClockwise, DotsThree, Power, Prohibit, ShieldCheck, Trash, WifiHigh } from '@phosphor-icons/react';
import Link from 'next/link';
import { useRouter, useSearchParams } from 'next/navigation';
import { Suspense, useEffect, useMemo, useState } from 'react';
import { Button, Card, Check, Chip, ConfirmDialog, Dialog, Empty, Field, GroupTag, HostStatusLabel, Menu, MenuItem, Notice, PageHeader, SearchInput, Select, Spinner, StatCard, StateIcon, Switch, Table, Tabs, Tag, TaskStatusLabel, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { MessageKey } from '@/lib/messages';
import { HostCompliance } from '@/components/Compliance';
import { Pager, usePageSize } from '@/components/Pager';
import { DockerList } from '@/components/DockerList';
import { HostResources, useSize } from '@/components/Metrics';
import { Occurrences, safeZone, useWindowText } from '@/components/PatchSchedule';
import type { Host, HostGroup, HostModules, HostPackage, PatchingExplain, Task } from '@/lib/types';

const HOST_MODULES: (keyof HostModules)[] = ['docker', 'helper_script', 'openscap', 'portal', 'portal_updates', 'eco'];
/** Module name -> what the agent reports in its facts when the host has it. */
const DETECTED: Partial<Record<keyof HostModules, string>> = { docker: 'docker', helper_script: 'helper-script', openscap: 'openscap' };

interface Detail { host: Host; group_ids: string[]; packages: HostPackage[]; tasks: Task[] }
type Draft = { display_name: string; environment: string; criticality: string; asset_type: string; internet_exposed: boolean; has_staging: boolean; ring: number; tags: string; group_ids: string[] };
type TabId = 'overview' | 'packages' | 'docker' | 'history' | 'compliance' | 'settings';

const draftOf = (d: Detail): Draft => ({ display_name: d.host.display_name ?? '', environment: d.host.environment, criticality: d.host.criticality, asset_type: d.host.asset_type, internet_exposed: d.host.internet_exposed, has_staging: d.host.has_staging, ring: d.host.ring, tags: d.host.tags.join(', '), group_ids: d.group_ids });

/** Automatic patching of one host: which policy applies, what is waiting, and the next patch day. */
function PatchingCard({ hostId }: { hostId: string }) {
  const { t, tr, date } = useI18n();
  const text = useWindowText();
  const e = useApi<PatchingExplain>(`/hosts/${hostId}/patching`, { poll: 20000 });
  if (!e.data) return null;
  const x = e.data;
  const chosen = x.policies.find((p) => p.id === x.chosen);
  const next = x.window?.next[0];
  const tz = safeZone(x.window?.config.timezone);
  const when = next ? date(next.start, { timeZone: tz, weekday: 'long', day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' }) : '';
  const tone = x.state === 'waiting_for_window' || x.state === 'will_plan' ? 'info' : x.state === 'nothing_pending' || x.state === 'task_open' ? 'ok' : 'warn';
  return (
    <Card title={t('host.patching')}>
      <div className="stack">
        <Notice kind={tone === 'info' ? 'info' : tone === 'ok' ? 'ok' : 'error'}>{t(`host.patching.${x.state}` as MessageKey, { n: x.pending.total, date: when, policy: chosen?.name ?? '' })}</Notice>
        {x.open_task && <p className="muted"><Link href={`/tasks?task=${x.open_task.id}`}>{tr('kind', x.open_task.kind)}</Link> · {tr('task', x.open_task.status)} · {date(x.open_task.scheduled_for)}</p>}
        {x.policies.length > 0 && (
          <div>
            <p className="label" style={{ marginBottom: 6 }}>{t('host.patching.policies')}</p>
            <ul style={{ margin: 0, paddingLeft: 20, fontSize: 13 }}>
              {x.policies.map((p) => (
                <li key={p.id} style={p.id === x.chosen ? { fontWeight: 500 } : undefined}>
                  {p.name} — {!p.in_scope ? t('host.patching.out_scope') : t('host.patching.matches', { n: p.matching })}
                  {p.security_only && p.not_security > 0 ? ` (${t('host.patching.sec_only', { n: p.not_security })})` : ''}
                  {p.docker && p.docker !== 'include' ? ` · ${t(`pol.docker.${p.docker}.short` as MessageKey)}${p.not_docker ? ` (${t('host.patching.docker_left', { n: p.not_docker })})` : ''}` : ''}
                  {p.window ? ` · ${text(p.window)} ${p.window.start}–${p.window.end}` : ''}
                </li>
              ))}
            </ul>
          </div>
        )}
        {x.window && x.window.next.length > 0 && (
          <div>
            <p className="label" style={{ marginBottom: 6 }}>{t('pol.patchday.next')}{x.window.open ? ` · ${t('pol.in_window')}` : ''}</p>
            <Occurrences list={x.window.next} zone={x.window.config.timezone} />
          </div>
        )}
      </div>
    </Card>
  );
}

function HostDetail() {
  const { t, tr, ago, date } = useI18n();
  const size = useSize();
  const { can } = useAuth();
  const toast = useToast();
  const router = useRouter();
  const params = useSearchParams();
  const id = params.get('id');
  const detail = useApi<Detail>(id ? `/hosts/${id}` : null, { poll: 8000 });
  const groups = useApi<HostGroup[]>(can('groups:read') ? '/groups' : null);
  const server = useApi<{ version: string }>('/server-info');

  const initialTab = (params.get('tab') as TabId | null) ?? 'overview';
  const [tab, setTab] = useState<TabId>(['overview', 'packages', 'docker', 'history', 'compliance', 'settings'].includes(initialTab) ? initialTab : 'overview');
  const [draft, setDraft] = useState<Draft | null>(null);
  const [dirty, setDirty] = useState(false);
  const [updatesOnly, setUpdatesOnly] = useState(true);
  const [securityOnly, setSecurityOnly] = useState(false);
  const [pkgQuery, setPkgQuery] = useState('');
  const [pkgSource, setPkgSource] = useState('');
  const [pkgPage, setPkgPage] = useState(0);
  const [perPage, setPerPage] = usePageSize();
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState<{ text: string; run: () => Promise<unknown>; danger?: boolean } | null>(null);
  const act = useAction();
  const [reboot, setReboot] = useState<{ delay: number; message: string } | null>(null);

  // Re-seed the form from the server, but never overwrite what the user is typing.
  useEffect(() => {
    if (detail.data && !dirty) setDraft(draftOf(detail.data));
  }, [detail.data, dirty]);

  const pkgs = useMemo(() => {
    const q = pkgQuery.trim().toLowerCase();
    return (detail.data?.packages ?? []).filter((p) => (!updatesOnly || p.candidate) && (!securityOnly || p.is_security) && (!pkgSource || p.source === pkgSource) && (!q || p.name.toLowerCase().includes(q)));
  }, [detail.data, updatesOnly, securityOnly, pkgQuery, pkgSource]);

  if (!id || detail.error?.status === 404) return <Notice kind="error">{t('host.notfound')}</Notice>;
  if (!detail.data || !draft) return detail.error ? <Notice kind="error">{detail.error.message}</Notice> : <Spinner />;

  const { host, packages, tasks } = detail.data;
  const facts = host.facts as Record<string, unknown>;
  const hw = facts.hardware as { cpu_model?: string; cpu_cores?: number; mem_total_mb?: number; swap_total_mb?: number; vendor?: string; product?: string; bios?: string } | undefined;
  const ips = Array.isArray(facts.ip_addresses) ? (facts.ip_addresses as string[]).join(', ') : '';
  const pending = packages.filter((p) => p.candidate);
  const sources = [...new Set(packages.map((p) => p.source))].filter((s) => ['system', 'docker', 'helper-script'].includes(s)).sort();
  const security = pending.filter((p) => p.is_security);
  const quarantined = host.status === 'quarantined';
  const canWrite = can('hosts:write');
  const caps = (facts.capabilities as string[] | undefined) ?? [];
  const detected = (facts.modules as string[] | undefined) ?? [];
  const agentVersion = String(facts.agent_version ?? '');
  const latest = server.data?.version ?? '';
  const outdated = !!latest && agentVersion !== latest;
  const updating = tasks.some((k) => k.kind === 'agent_update' && ['queued', 'sent', 'running'].includes(k.status));
  const setModule = (m: keyof HostModules, on: boolean) => act.run(async () => {
    await api('PATCH', `/hosts/${id}`, { modules: { [m]: on } });
    toast.success(t('common.saved'));
    detail.reload();
  });
  const updateAgent = () => {
    if (!caps.includes('self_update')) { toast.error(t('host.agent.update.old')); return; }
    setConfirm({ text: t('host.agent.update.confirm', { v: latest }), run: () => doTask(`/hosts/${id}/agent-update`) });
  };
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) => { setDraft({ ...draft, [k]: v }); setDirty(true); };

  const save = () => act.run(async () => {
    await api('PATCH', `/hosts/${id}`, {
      display_name: draft.display_name, environment: draft.environment, criticality: draft.criticality, asset_type: draft.asset_type, internet_exposed: draft.internet_exposed,
      has_staging: draft.has_staging, ring: Number(draft.ring), tags: draft.tags.split(',').map((x) => x.trim()).filter(Boolean), group_ids: draft.group_ids,
    });
    setDirty(false);
    toast.success(t('common.saved'));
    detail.reload();
  });
  const doTask = (path: string, body?: unknown) => act.run(async () => {
    await api('POST', path, body);
    toast.success(t('host.queued'));
    detail.reload();
  });
  const patch = (names: string[]) => setConfirm({ text: t('host.confirm_patch'), run: () => doTask('/tasks', { host_ids: [id], packages: names }) });
  const changeTab = (next: TabId) => { setTab(next); router.replace(`/host?id=${id}&tab=${next}`); };

  const PkgTable = ({ rows, selectable }: { rows: HostPackage[]; selectable: boolean }) => (
    <Table head={[selectable ? '' : null, t('host.pkg.name'), t('host.pkg.installed'), t('host.pkg.candidate'), t('host.pkg.cves')].filter((h) => h !== null)} empty={rows.length === 0 && <Empty>{t('host.pkg.none')}</Empty>}>
      {rows.map((p) => (
        <tr key={`${p.source}/${p.name}`}>
          {selectable && <td>{p.candidate && can('tasks:create') && <input type="checkbox" aria-label={p.name} checked={picked.has(p.name)} onChange={(e) => { const n = new Set(picked); if (e.target.checked) n.add(p.name); else n.delete(p.name); setPicked(n); }} style={{ accentColor: 'var(--primary)' }} />}</td>}
          <td className="strong">{p.name} {p.is_security && <Tag tone="brick">{t('pkg.security')}</Tag>}</td>
          <td>{p.version}</td>
          <td>{p.candidate ?? <span className="muted">—</span>}</td>
          <td className="wrap">{p.cves.length ? <div className="pkgs">{p.cves.map((c) => <Tag key={c}>{c}</Tag>)}</div> : <span className="muted">—</span>}</td>
        </tr>
      ))}
    </Table>
  );

  return (
    <>
      <PageHeader
        back={{ href: '/hosts', label: t('nav.hosts') }}
        title={host.display_name || host.hostname}
        badge={<><HostStatusLabel status={host.status} /><Tag>{tr('env', host.environment)}</Tag></>}
        subtitle={`${host.distro} ${String(facts.os_version ?? '')} · ${String(facts.kernel ?? '')}`}
        actions={<>
          {can('tasks:create') && <Button icon={<ArrowsClockwise size={16} weight="bold" />} disabled={act.pending || quarantined} onClick={() => void doTask(`/hosts/${id}/scan`)}>{t('host.scan')}</Button>}
          {can('tasks:create') && !quarantined && pending.length > 0 && <Button variant="primary" icon={<ShieldCheck size={16} weight="bold" />} disabled={act.pending} onClick={() => patch([])}>{t('host.patch_all')}</Button>}
          {(can('tasks:write') || can('hosts:delete') || canWrite) && (
            <Menu label={t('host.more')} trigger={<DotsThree size={20} weight="bold" />}>
              {(close) => (
                <>
                  {canWrite && !quarantined && <MenuItem icon={<ArrowCircleUp size={16} weight="bold" />} onClick={() => { close(); updateAgent(); }}>{t('host.agent.update')}</MenuItem>}
                  {can('tasks:write') && !quarantined && <MenuItem icon={<Power size={16} weight="bold" />} onClick={() => { close(); setReboot({ delay: host.asset_type === 'server' ? 0 : 10, message: '' }); }}>{t('host.reboot')}</MenuItem>}
                  {can('tasks:write') && (quarantined
                    ? <MenuItem icon={<WifiHigh size={16} weight="bold" />} onClick={() => { close(); void doTask(`/hosts/${id}/release`); }}>{t('host.release')}</MenuItem>
                    : <MenuItem icon={<Prohibit size={16} weight="bold" />} onClick={() => { close(); setConfirm({ text: t('host.confirm_quarantine'), run: () => doTask(`/hosts/${id}/quarantine`), danger: true }); }}>{t('host.quarantine')}</MenuItem>)}
                  {can('hosts:delete') && <><div className="menu__sep" /><MenuItem danger icon={<Trash size={16} weight="bold" />} onClick={() => { close(); setConfirm({ text: t('host.confirm_delete'), danger: true, run: async () => { await api('DELETE', `/hosts/${id}`); router.replace('/hosts/'); } }); }}>{t('host.delete')}</MenuItem></>}
                </>
              )}
            </Menu>
          )}
        </>}
      />
      {act.error && <div style={{ marginBottom: 16 }}><Notice kind="error">{act.error}</Notice></div>}
      {host.status === 'maintenance' && host.downtime_until && <div style={{ marginBottom: 16 }}><Notice>{t('host.reboot.downtime', { t: date(host.downtime_until) })}</Notice></div>}
      {host.reboot_required && host.status !== 'maintenance' && !quarantined && (
        <div className="alert alert--warn" role="status" style={{ marginBottom: 16, alignItems: 'center' }}>
          <StateIcon tone="warn" size={20} />
          <div className="alert__body">{t('host.reboot.needed')}</div>
          {can('tasks:write') && <Button small icon={<Power size={16} weight="bold" />} onClick={() => setReboot({ delay: host.asset_type === 'server' ? 0 : 10, message: '' })}>{t('host.reboot')}</Button>}
        </div>
      )}

      <Tabs value={tab} onChange={changeTab} tabs={[
        { id: 'overview', label: t('host.tab.overview') },
        { id: 'packages', label: t('host.tab.packages'), count: pending.length || undefined },
        ...(host.modules?.docker ? [{ id: 'docker' as const, label: t('host.tab.docker'), count: packages.filter((p) => p.source === 'docker').length || undefined }] : []),
        { id: 'history', label: t('host.tab.history'), count: tasks.length || undefined },
        { id: 'compliance', label: t('host.tab.compliance') },
        { id: 'settings', label: t('host.tab.settings') },
      ]} />

      {tab === 'overview' && (
        <div className="stack stack--lg">
          <div className="grid grid--stats">
            <StatCard value={pending.length} label={t('host.stat.updates')} />
            <StatCard value={security.length} label={t('host.stat.security')} alert={security.length > 0} />
            <StatCard value={ago(host.last_seen_at)} label={t('host.stat.seen')} />
            <StatCard value={host.reboot_required ? t('host.reboot.yes') : t('host.reboot.no')} label={t('host.stat.reboot')} alert={host.reboot_required} />
          </div>
          <div className="grid grid--2">
            <Card title={t('host.section.system')}>
              <dl className="kv">
                <dt>{t('host.os')}</dt><dd>{host.distro} {String(facts.os_version ?? '')}</dd>
                <dt>{t('host.kernel')}</dt><dd>{String(facts.kernel ?? '—')}</dd>
                <dt>{t('host.arch')}</dt><dd>{String(facts.arch ?? '—')}</dd>
                <dt>{t('host.pkgmgr')}</dt><dd>{host.package_manager}</dd>
                {hw?.cpu_model && <><dt>{t('host.hw.cpu')}</dt><dd>{hw.cpu_model}{hw.cpu_cores ? ` · ${t('host.hw.cores', { n: hw.cpu_cores })}` : ''}</dd></>}
                {!!hw?.mem_total_mb && <><dt>{t('host.hw.memory')}</dt><dd>{size(hw.mem_total_mb)}{hw.swap_total_mb ? ` · swap ${size(hw.swap_total_mb)}` : ''}</dd></>}
                {(hw?.vendor || hw?.product) && <><dt>{t('host.hw.model')}</dt><dd>{[hw.vendor, hw.product].filter(Boolean).join(' ')}</dd></>}
                {hw?.bios && <><dt>BIOS</dt><dd>{hw.bios}</dd></>}
                {typeof facts.virtualization === 'string' && facts.virtualization && <><dt>{t('host.hw.virt')}</dt><dd>{facts.virtualization}</dd></>}
                <dt>{t('host.asset_type')}</dt><dd>{tr('asset', host.asset_type)}{facts.is_desktop && host.asset_type === 'server' ? <> <Tag tone="info">{t('host.desktop_detected')}</Tag></> : null}</dd>
                {ips && <><dt>{t('host.ips')}</dt><dd>{ips}</dd></>}
              </dl>
            </Card>
            <Card title={t('host.section.agent')}>
              <dl className="kv">
                <dt>{t('host.agent')}</dt>
                <dd className="row row--tight">
                  {agentVersion || '—'}
                  {updating ? <Tag tone="info">{t('portal.working')}</Tag> : outdated ? <Tag tone="warn">{t('host.agent.outdated', { v: latest })}</Tag> : latest ? <Tag tone="ok">{t('host.agent.current')}</Tag> : null}
                  {canWrite && outdated && !updating && !quarantined && <Button small icon={<ArrowCircleUp size={14} weight="bold" />} disabled={act.pending} onClick={updateAgent}>{t('host.agent.update')}</Button>}
                </dd>
                <dt>{t('host.comm')}</dt><dd>{t(host.comm_mode === 'pull' ? 'hosts.mode.pull' : 'hosts.mode.push')}{host.connected ? ' · ●' : ''}</dd>
                <dt>{t('host.last_seen')}</dt><dd>{date(host.last_seen_at)}</dd>
                <dt>{t('host.last_inventory')}</dt><dd>{date(host.last_inventory_at)}</dd>
                <dt>{t('host.portal_client')}</dt><dd>{t(facts.portal_gui ? 'host.portal_client.gui' : 'host.portal_client.tui')}</dd>
                {host.eco && <><dt>Eco</dt><dd>{t('host.eco_note')}</dd></>}
                {host.downtime_until && <><dt>{t('host.downtime')}</dt><dd>{date(host.downtime_until)}</dd></>}
              </dl>
            </Card>
          </div>
          <HostResources hostId={id} />
          <PatchingCard hostId={id} />
          <Card title={t('host.pending_security')} actions={pending.length > 0 && <Button small variant="ghost" onClick={() => changeTab('packages')}>{t('host.tab.packages')} →</Button>} flush>
            {security.length === 0 ? <Empty>{t('host.no_pending')}</Empty> : <PkgTable rows={security.slice(0, 8)} selectable={false} />}
          </Card>
        </div>
      )}

      {tab === 'packages' && (
        <>
          <div className="toolbar">
            <SearchInput value={pkgQuery} onChange={(v) => { setPkgQuery(v); setPkgPage(0); }} placeholder={t('host.pkg.search')} />
            <Chip active={updatesOnly} onClick={() => { setUpdatesOnly((v) => !v); setPkgPage(0); }}>{t('host.pending_only')}</Chip>
            <Chip active={securityOnly} onClick={() => { setSecurityOnly((v) => !v); setPkgPage(0); }}>{t('host.security_only')}</Chip>
            {sources.length > 1 && sources.map((s) => <Chip key={s} active={pkgSource === s} onClick={() => { setPkgSource(pkgSource === s ? '' : s); setPkgPage(0); }}>{t(`host.pkg.source.${s}` as MessageKey)} · {packages.filter((p) => p.source === s).length}</Chip>)}
            {can('tasks:create') && !quarantined && pending.length > 0 && (
              <span className="row row--tight right">
                <Button small disabled={picked.size === 0 || act.pending} onClick={() => patch([...picked])}>{t('host.patch_selected')}{picked.size ? ` (${picked.size})` : ''}</Button>
              </span>
            )}
          </div>
          <PkgTable rows={pkgs.slice(pkgPage * perPage, (pkgPage + 1) * perPage)} selectable />
          <Pager page={Math.min(pkgPage, Math.max(0, Math.ceil(pkgs.length / perPage) - 1))} perPage={perPage} total={pkgs.length} onPage={setPkgPage} onPerPage={setPerPage} note={!updatesOnly && !pkgQuery && !pkgSource ? t('host.pkg.all_installed', { n: packages.length }) : undefined} />
        </>
      )}

      {tab === 'docker' && <DockerList hostId={id} />}

      {tab === 'history' && (
        <Table head={[t('tasks.col.task'), t('common.status'), t('tasks.col.packages'), t('tasks.col.by'), t('tasks.col.when')]} empty={tasks.length === 0 && <Empty>{t('tasks.none')}</Empty>}>
          {tasks.map((k) => (
            <tr key={k.id}>
              <td><Link href={`/tasks?task=${k.id}`}>{tr('kind', k.kind)}</Link></td>
              <td><TaskStatusLabel status={k.status} /></td>
              <td>{k.packages.length || '—'}</td>
              <td className="wrap">{k.requested_by}</td>
              <td className="nowrap">{ago(k.updated_at)}</td>
            </tr>
          ))}
        </Table>
      )}

      {tab === 'compliance' && <HostCompliance hostId={id} modules={(facts.modules as string[] | undefined) ?? []} canScan={can('tasks:create') && !quarantined} onScan={() => void doTask(`/hosts/${id}/compliance-scan`, {})} />}
      {tab === 'settings' && (
        <div className="stack stack--lg">
        <Card title={t('host.modules')}>
          <p className="muted" style={{ marginBottom: 16 }}>{t('host.modules.sub')}</p>
          {!caps.includes('settings') && <div style={{ marginBottom: 16 }}><Notice>{t('host.modules.old_agent')}</Notice></div>}
          <div className="stack" style={{ gap: 14 }}>
            {HOST_MODULES.map((m) => {
              const probe = DETECTED[m];
              return (
                <div key={m} className="row" style={{ justifyContent: 'space-between', flexWrap: 'nowrap' }}>
                  <div>
                    <b>{t(`mod.${m}` as MessageKey)}</b> {probe && <Tag tone={detected.includes(probe) ? 'ok' : 'neutral'}>{t(detected.includes(probe) ? 'host.modules.detected' : 'host.modules.not_detected')}</Tag>}
                    <p className="field__hint">{t(`mod.${m}.desc` as MessageKey)}</p>
                  </div>
                  <Switch checked={!!host.modules?.[m]} disabled={!canWrite || act.pending} onChange={(v) => void setModule(m, v)}><span className="sr-only">{t(`mod.${m}` as MessageKey)}</span></Switch>
                </div>
              );
            })}
          </div>
        </Card>
        <Card title={t('host.section.attributes')}>
          <p className="muted" style={{ marginBottom: 16 }}>{t('host.section.attributes.sub')}</p>
          <form className="form-grid" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <Field label={t('common.name')}>{(i) => <TextInput id={i} value={draft.display_name} placeholder={host.hostname} disabled={!canWrite} onChange={(e) => set('display_name', e.target.value)} />}</Field>
            <Field label={t('host.environment')}>{(i) => <Select id={i} value={draft.environment} disabled={!canWrite} onChange={(e) => set('environment', e.target.value)}>{['production', 'staging', 'dev'].map((v) => <option key={v} value={v}>{tr('env', v)}</option>)}</Select>}</Field>
            <Field label={t('host.criticality')}>{(i) => <Select id={i} value={draft.criticality} disabled={!canWrite} onChange={(e) => set('criticality', e.target.value)}>{['low', 'medium', 'high'].map((v) => <option key={v} value={v}>{tr('crit', v)}</option>)}</Select>}</Field>
            <Field label={t('host.asset_type')}>{(i) => <Select id={i} value={draft.asset_type} disabled={!canWrite} onChange={(e) => set('asset_type', e.target.value)}>{['server', 'workstation', 'laptop'].map((v) => <option key={v} value={v}>{tr('asset', v)}</option>)}</Select>}</Field>
            <Field label={t('host.ring')} hint={t('host.ring.hint')}>{(i) => <TextInput id={i} type="number" min={0} max={10} value={draft.ring} disabled={!canWrite} onChange={(e) => set('ring', Number(e.target.value))} />}</Field>
            <Field label={t('host.tags')} hint={t('host.tags.hint')}>{(i) => <TextInput id={i} value={draft.tags} disabled={!canWrite} onChange={(e) => set('tags', e.target.value)} />}</Field>
            <div className="span-all row">
              <Check checked={draft.internet_exposed} disabled={!canWrite} onChange={(v) => set('internet_exposed', v)}>{t('host.internet_exposed')}</Check>
              <Check checked={draft.has_staging} disabled={!canWrite} onChange={(v) => set('has_staging', v)}>{t('host.has_staging')}</Check>
            </div>
            {groups.data && (
              <fieldset className="fieldset span-all">
                <legend>{t('host.groups')}</legend>
                <div className="chips">{groups.data.map((g) => <GroupTag key={g.id} group={g} active={draft.group_ids.includes(g.id)} disabled={!canWrite} onClick={() => set('group_ids', draft.group_ids.includes(g.id) ? draft.group_ids.filter((x) => x !== g.id) : [...draft.group_ids, g.id])} />)}</div>
              </fieldset>
            )}
            {canWrite && <div className="span-all"><Button variant="primary" type="submit" disabled={!dirty || act.pending}>{t('common.save')}</Button></div>}
          </form>
        </Card>
        </div>
      )}

      <Dialog open={!!reboot} title={t('host.reboot.title')} onClose={() => setReboot(null)}>
        {reboot && (
          <form className="stack" onSubmit={(e) => { e.preventDefault(); const r = reboot; setReboot(null); void doTask(`/hosts/${id}/reboot`, { delay_minutes: r.delay, message: r.message }); }}>
            <Field label={t('host.reboot.delay')} hint={t('host.reboot.delay_hint')}>{(i) => <TextInput id={i} type="number" min={0} max={1440} value={reboot.delay} onChange={(e) => setReboot({ ...reboot, delay: Math.max(0, Number(e.target.value)) })} />}</Field>
            <Field label={t('host.reboot.message')}>{(i) => <TextInput id={i} maxLength={200} value={reboot.message} onChange={(e) => setReboot({ ...reboot, message: e.target.value })} />}</Field>
            <div className="row"><Button variant="danger" type="submit" icon={<Power size={16} weight="bold" />}>{t('host.reboot.go')}</Button><Button onClick={() => setReboot(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Dialog>
      <ConfirmDialog message={confirm?.text ?? null} danger={confirm?.danger} onNo={() => setConfirm(null)} onYes={() => { const c = confirm; setConfirm(null); if (c) void act.run(c.run); }} />
    </>
  );
}

export default function Page() {
  return (
    <>
      <Suspense fallback={<Spinner />}>
        <HostDetail />
      </Suspense>
    </>
  );
}
