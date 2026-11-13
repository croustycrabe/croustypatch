'use client';
import { ArrowCounterClockwise, ArrowDown, ArrowRight, ArrowUp, CheckCircle, Eye, EyeSlash, SlidersHorizontal } from '@phosphor-icons/react';
import Link from 'next/link';
import { useEffect, useState, type ReactNode } from 'react';
import { AddAgentButton } from '@/components/AddAgent';
import { Meter } from '@/components/Metrics';
import { Button, Card, Chip, Empty, Notice, PageHeader, Spinner, StatCard, StatusDot, Tag, TaskStatusLabel, HOST_DOT, HOST_TONE } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import type { Dashboard, HostStatus, Layout } from '@/lib/types';

type Size = 'xs' | 'sm' | 'md' | 'lg';
type Widget = { id: string; visible: boolean; size: Size };
type Def = { kind: 'stat' | 'panel'; title: MessageKey; size: Size; visible: boolean };

/** Everything that can be on the dashboard, in the order a new dashboard shows it. `visible` is the default. */
const CATALOG: Record<string, Def> = {
  stat_hosts: { kind: 'stat', title: 'dash.cards.hosts', size: 'xs', visible: true },
  stat_needing: { kind: 'stat', title: 'dash.cards.needing', size: 'xs', visible: true },
  stat_security: { kind: 'stat', title: 'dash.cards.security', size: 'xs', visible: true },
  stat_outdated: { kind: 'stat', title: 'dash.cards.outdated', size: 'xs', visible: true },
  stat_attention: { kind: 'stat', title: 'dash.cards.attention', size: 'xs', visible: true },
  stat_uncovered: { kind: 'stat', title: 'dash.cards.uncovered', size: 'xs', visible: true },
  stat_reboot: { kind: 'stat', title: 'dash.cards.reboot', size: 'xs', visible: false },
  stat_approvals: { kind: 'stat', title: 'dash.cards.approvals', size: 'xs', visible: false },
  stat_patched_week: { kind: 'stat', title: 'dash.cards.patched_week', size: 'xs', visible: false },
  stat_failed_week: { kind: 'stat', title: 'dash.cards.failed_week', size: 'xs', visible: false },
  fleet: { kind: 'panel', title: 'w.fleet', size: 'md', visible: true },
  updates: { kind: 'panel', title: 'w.updates', size: 'md', visible: true },
  patching: { kind: 'panel', title: 'w.patching', size: 'md', visible: true },
  resources: { kind: 'panel', title: 'w.resources', size: 'md', visible: true },
  packages: { kind: 'panel', title: 'w.packages', size: 'md', visible: true },
  vulns: { kind: 'panel', title: 'w.vulns', size: 'md', visible: true },
  os: { kind: 'panel', title: 'w.os', size: 'md', visible: true },
  activity: { kind: 'panel', title: 'w.activity', size: 'lg', visible: true },
  portal: { kind: 'panel', title: 'w.portal', size: 'md', visible: true },
};
const IDS = Object.keys(CATALOG);
const SIZES: Size[] = ['xs', 'sm', 'md', 'lg'];
const defaults = (): Widget[] => IDS.map((id) => ({ id, visible: CATALOG[id]!.visible, size: CATALOG[id]!.size }));

// Segment colours come from the palette: ink for healthy, brick for trouble, tints in between.
const BAR: Record<HostStatus, string> = {
  online: 'var(--success)', eco: 'var(--info)', maintenance: 'var(--warning)', offline: 'var(--border-strong)', alert: 'var(--danger)', quarantined: 'var(--danger)', pending: 'var(--surface-active)',
};

/** Keep only known widgets, in the saved order. A layout saved before the indicator tiles existed gets the five
 * original ones back at the top (they used to be fixed); anything else it predates is added hidden, so nothing
 * moves under the user: it is one click away in the customise view. */
function normalise(saved: Layout | undefined): Widget[] {
  const known = (saved?.widgets ?? []).filter((w): w is Widget => w.id in CATALOG).map((w) => ({ ...w, size: (SIZES as string[]).includes(w.size) ? w.size : CATALOG[w.id]!.size }));
  const hasTiles = known.some((w) => CATALOG[w.id]!.kind === 'stat');
  const originals = ['stat_hosts', 'stat_needing', 'stat_security', 'stat_outdated', 'stat_attention'];
  const head = hasTiles || known.length === 0 ? [] : originals.map((id) => ({ id, visible: true, size: 'xs' as Size }));
  const have = new Set([...head, ...known].map((w) => w.id));
  const missing = IDS.filter((id) => !have.has(id)).map((id) => ({ id, visible: known.length === 0 ? CATALOG[id]!.visible : false, size: CATALOG[id]!.size }));
  return [...head, ...known, ...missing];
}

function ModuleOff() {
  const { t } = useI18n();
  const { can } = useAuth();
  return (
    <div className="module-off">
      <p className="label">{t('w.module_off')}</p>
      <p className="muted" style={{ fontSize: 13 }}>{t('w.module_off_hint')}</p>
      {can('settings:read') && <Link href="/settings">{t('nav.settings')}</Link>}
    </div>
  );
}

function Tile({ id, d }: { id: string; d: Dashboard }) {
  const { t } = useI18n();
  const label = t(CATALOG[id]!.title);
  const attentionHosts = (d.fleet.by_status.alert ?? 0) + (d.fleet.by_status.quarantined ?? 0) + (d.fleet.by_status.offline ?? 0);
  const failedWeek = d.patching.week.failed + d.patching.week.rolled_back + d.patching.week.missed;
  switch (id) {
    case 'stat_hosts': return <StatCard value={d.fleet.total} label={label} href="/hosts" />;
    case 'stat_needing': return <StatCard value={d.fleet.hosts_needing_updates} tone={d.fleet.hosts_needing_updates > 0 ? 'warn' : 'ok'} label={label} href="/hosts?updates=1" />;
    case 'stat_security': return <StatCard value={d.vulnerabilities.security_updates} label={label} alert={d.vulnerabilities.security_updates > 0} href="/hosts?security=1" />;
    case 'stat_outdated': return <StatCard value={d.vulnerabilities.outdated_packages} tone={d.vulnerabilities.outdated_packages > 0 ? 'warn' : 'ok'} label={label} href="/packages?updates=1" />;
    case 'stat_attention': return <StatCard value={attentionHosts} label={label} alert={attentionHosts > 0} href="/hosts?attention=1" />;
    case 'stat_uncovered': return <StatCard value={d.patching.uncovered_hosts} tone={d.patching.uncovered_hosts > 0 ? 'warn' : 'ok'} label={label} href="/policies" />;
    case 'stat_reboot': return <StatCard value={d.fleet.reboot_required} tone={d.fleet.reboot_required > 0 ? 'warn' : 'ok'} label={label} href="/hosts?reboot=1" />;
    case 'stat_approvals': return <StatCard value={d.tasks.awaiting_approval} tone={d.tasks.awaiting_approval > 0 ? 'warn' : 'ok'} label={label} href="/tasks?filter=approval" />;
    case 'stat_patched_week': return <StatCard value={d.patching.week.succeeded} tone="ok" label={label} href="/tasks" />;
    case 'stat_failed_week': return <StatCard value={failedWeek} alert={failedWeek > 0} label={label} href="/tasks" />;
    default: return null;
  }
}

function WidgetBody({ id, d }: { id: string; d: Dashboard }) {
  const { t, tr, ago, date } = useI18n();
  const total = Math.max(d.fleet.total, 1);
  switch (id) {
    case 'updates': {
      const needing = d.fleet.hosts_needing_updates;
      const sec = Math.min(d.vulnerabilities.hosts_with_security_updates, needing);
      const ok = Math.max(d.fleet.total - needing, 0);
      const seg = [{ k: 'ok', n: ok, c: 'var(--success)', href: '/hosts' }, { k: 'ordinary', n: needing - sec, c: 'var(--warning)', href: '/hosts?updates=1' }, { k: 'security', n: sec, c: 'var(--danger)', href: '/hosts?security=1' }];
      return (
        <div>
          <div className="fleetbar" role="img" aria-label={seg.map((x) => `${t(`w.updates.${x.k}` as MessageKey)} ${x.n}`).join(', ')}>
            {seg.map((x) => <span key={x.k} style={{ flex: x.n, background: x.c }} />)}
          </div>
          <div className="legend">{seg.map((x) => <Link key={x.k} href={x.href}><span style={{ display: 'inline-block', width: 10, height: 10, borderRadius: 2, background: x.c, marginRight: 6 }} />{t(`w.updates.${x.k}` as MessageKey)} · {x.n}</Link>)}</div>
        </div>
      );
    }
    case 'resources': {
      const r = d.resources;
      if (r.reporting === 0) return <p className="muted">{t('w.resources.none')}</p>;
      return (
        <div className="stack" style={{ gap: 10 }}>
          <Meter label={t('w.resources.avg_cpu')} pct={r.avg_cpu} />
          <Meter label={t('w.resources.avg_mem')} pct={r.avg_mem} />
          <p className="label" style={{ margin: '6px 0 0' }}>{t('w.resources.busiest')}</p>
          <ul className="stack" style={{ listStyle: 'none', margin: 0, padding: 0, gap: 6 }}>
            {r.busiest.map((h) => (
              <li key={h.id} className="row" style={{ justifyContent: 'space-between', fontSize: 13, flexWrap: 'nowrap' }}>
                <Link href={`/host?id=${h.id}`} style={{ fontWeight: 600 }}>{h.hostname}</Link>
                <span className="row row--tight">
                  {([['cpu', h.cpu], ['ram', h.mem], ['disk', h.disk]] as const).map(([k, v]) => v == null ? null : <Tag key={k} tone={v >= 90 ? 'danger' : v >= 75 ? 'warn' : 'neutral'}>{t(`w.resources.${k}` as MessageKey)} {Math.round(v)}%</Tag>)}
                </span>
              </li>
            ))}
          </ul>
        </div>
      );
    }
    case 'patching': {
      const p = d.patching;
      return (
        <div className="stack" style={{ gap: 10 }}>
          <div className="row row--tight">
            <Tag tone="ok">{t('w.patching.ok', { n: p.week.succeeded })}</Tag>
            {p.week.failed > 0 && <Tag tone="danger">{t('w.patching.failed', { n: p.week.failed })}</Tag>}
            {p.week.rolled_back > 0 && <Tag tone="warn">{t('w.patching.rolled_back', { n: p.week.rolled_back })}</Tag>}
            {p.week.missed > 0 && <Tag tone="warn">{t('w.patching.missed', { n: p.week.missed })}</Tag>}
          </div>
          <p className="label" style={{ margin: '4px 0 0' }}>{t('pol.patchday.next')}</p>
          {p.next.length === 0 ? <p className="muted" style={{ margin: 0 }}>{t('w.patching.none')}</p> : (
            <ul className="stack" style={{ listStyle: 'none', margin: 0, padding: 0, gap: 6 }}>
              {p.next.map((n) => (
                <li key={n.policy} className="row" style={{ justifyContent: 'space-between', fontSize: 13, flexWrap: 'nowrap' }}>
                  <span><Link href="/policies" style={{ fontWeight: 600 }}>{n.policy}</Link><span className="sub">{date(n.start, { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</span></span>
                  <span className="row row--tight"><Tag tone={n.auto ? 'info' : 'neutral'}>{t(n.auto ? 'w.patching.auto' : 'w.patching.approval')}</Tag><span className="muted">{t('pol.col.hosts', { n: n.hosts })}</span></span>
                </li>
              ))}
            </ul>
          )}
          {p.uncovered_hosts > 0 && <Notice kind="error">{t('w.patching.uncovered', { n: p.uncovered_hosts })} <Link href="/policies">{t('nav.policies')}</Link></Notice>}
        </div>
      );
    }
    case 'fleet':
      return (
        <div>
          <div className="fleetbar" role="img" aria-label={Object.entries(d.fleet.by_status).map(([s, n]) => `${tr('status', s)} ${n}`).join(', ')}>
            {Object.entries(d.fleet.by_status).map(([s, n]) => <span key={s} style={{ flex: n, background: BAR[s as HostStatus] ?? 'var(--cream-3)' }} />)}
          </div>
          <div className="legend">
            {Object.entries(d.fleet.by_status).map(([s, n]) => (
              <Link key={s} href={`/hosts?status=${s}`}><StatusDot kind={HOST_DOT[s as HostStatus] ?? 'hollow'} tone={HOST_TONE[s as HostStatus] ?? 'neutral'} />{tr('status', s)} · {n}</Link>
            ))}
          </div>
        </div>
      );
    case 'os':
      return (
        <div className="stack" style={{ gap: 10 }}>
          {d.fleet.os.map((o) => (
            <div key={o.name}>
              <div className="row" style={{ justifyContent: 'space-between', gap: 8, fontSize: 13 }}><span>{o.name}</span><b>{o.count}</b></div>
              <div className="fleetbar" style={{ height: 6, margin: '4px 0 0' }}><span style={{ flex: o.count / total, background: 'var(--ink)' }} /><span style={{ flex: 1 - o.count / total, background: 'transparent' }} /></div>
            </div>
          ))}
        </div>
      );
    case 'packages':
      return d.top_outdated.length === 0 ? (
        <p className="muted">{t('w.packages.empty')}</p>
      ) : (
        <ul className="stack" style={{ listStyle: 'none', margin: 0, padding: 0, gap: 8 }}>
          {d.top_outdated.map((p) => (
            <li key={p.name} className="row" style={{ justifyContent: 'space-between', fontSize: 13 }}>
              <Link href={`/packages?q=${encodeURIComponent(p.name)}`} style={{ fontWeight: 600 }}>{p.name}</Link>
              <span className="row row--tight">{p.security && <Tag tone="brick">{t('pkg.security')}</Tag>}<span className="muted">{p.hosts === 1 ? t('w.hosts_one') : t('w.hosts_n', { n: p.hosts })}</span></span>
            </li>
          ))}
        </ul>
      );
    case 'vulns':
      return (
        <div className="grid grid--stats">
          <StatCard value={d.vulnerabilities.critical_cves_unpatched} label={t('w.vulns.critical')} alert={d.vulnerabilities.critical_cves_unpatched > 0} href="/vulnerabilities" />
          <StatCard value={d.vulnerabilities.hosts_with_security_updates} label={t('w.vulns.hosts')} />
        </div>
      );
    case 'activity':
      return d.activity.length === 0 ? (
        <Empty>{t('w.activity.empty')}</Empty>
      ) : (
        <ul className="activity">
          {d.activity.map((a) => (
            <li key={`${a.id}-${a.status}`}>
              <span><Link href={`/host?id=${a.host_id}`} style={{ fontWeight: 600 }}>{a.hostname}</Link> · {tr('kind', a.kind)}</span>
              <TaskStatusLabel status={a.status} />
              <small>{ago(a.ts)}{a.message ? ` · ${a.message}` : ''}</small>
            </li>
          ))}
        </ul>
      );
    case 'portal':
      return d.portal.enabled ? <StatCard value={d.portal.installs ?? '—'} label={t('w.portal')} /> : <ModuleOff />;
  }
}

function Dashboard() {
  const { t } = useI18n();
  const { can } = useAuth();
  const dash = useApi<Dashboard>('/dashboard', { poll: 10000 });
  const saved = useApi<Layout>('/dashboard/layout');
  const [widgets, setWidgets] = useState<Widget[]>([]);
  const [editing, setEditing] = useState(false);
  const [dragId, setDragId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const [saveError, setSaveError] = useState(false);

  useEffect(() => {
    if (saved.data) setWidgets(normalise(saved.data));
  }, [saved.data]);

  const persist = (next: Widget[]) => {
    setWidgets(next);
    setSaveError(false);
    api('PUT', '/dashboard/layout', { widgets: next }).catch(() => setSaveError(true));
  };
  const move = (id: string, delta: number) => {
    const i = widgets.findIndex((w) => w.id === id);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= widgets.length) return;
    const next = [...widgets];
    const [w] = next.splice(i, 1);
    next.splice(j, 0, w!);
    persist(next);
  };
  const drop = (target: string) => {
    if (dragId && dragId !== target) {
      const next = widgets.filter((w) => w.id !== dragId);
      const at = next.findIndex((w) => w.id === target);
      next.splice(at, 0, widgets.find((w) => w.id === dragId)!);
      persist(next);
    }
    setDragId(null);
    setOverId(null);
  };
  const patch = (id: string, p: Partial<Widget>) => persist(widgets.map((w) => (w.id === id ? { ...w, ...p } : w)));
  // A widget that was hidden comes back where it belongs in the layout, which may be far from the list it was shown
  // from: bring it into view.
  const reveal = (id: string) => {
    patch(id, { visible: true });
    setTimeout(() => document.querySelector(`[data-widget="${id}"]`)?.scrollIntoView({ block: 'center' }), 80);
  };

  const d = dash.data;
  const visible = widgets.filter((w) => w.visible);
  const hidden = widgets.filter((w) => !w.visible);
  const attentionHosts = d ? (d.fleet.by_status.alert ?? 0) + (d.fleet.by_status.quarantined ?? 0) + (d.fleet.by_status.offline ?? 0) : 0;
  const attention = d
    ? [
        { n: d.tasks.awaiting_approval, label: t('attn.approvals'), href: '/tasks?filter=approval', warn: true },
        { n: d.fleet.by_status.quarantined ?? 0, label: t('attn.quarantined'), href: '/hosts?status=quarantined', warn: false },
        { n: d.vulnerabilities.critical_cves_unpatched, label: t('attn.critical'), href: '/vulnerabilities', warn: false },
        { n: d.tasks.waivers_pending, label: t('attn.waivers'), href: '/waivers', warn: true },
        { n: d.fleet.reboot_required, label: t('attn.reboot'), href: '/hosts?reboot=1', warn: true },
      ].filter((a) => a.n > 0)
    : [];

  return (
    <>
      <PageHeader title={t('dash.title')} subtitle={t('dash.subtitle')}
        actions={<>
          {d && d.fleet.total > 0 && <Button variant={editing ? 'ink' : 'outline'} icon={<SlidersHorizontal size={16} weight="bold" />} onClick={() => setEditing((v) => !v)}>{editing ? t('dash.done') : t('dash.customize')}</Button>}
          <AddAgentButton />
        </>} />
      {saveError && <Notice kind="error">{t('common.error')}</Notice>}
      {!d ? (
        dash.error ? <Notice kind="error">{dash.error.message}</Notice> : <Spinner />
      ) : d.fleet.total === 0 ? (
        <Card>
          <Empty title={t('dash.welcome.title')} action={<AddAgentButton />}>{t('dash.welcome.text')}</Empty>
        </Card>
      ) : (
        <div className="stack stack--lg">
          {attention.length > 0 ? (
            <div className="attention" style={{ margin: 0 }}>
              {attention.map((a) => (
                <Link key={a.href} href={a.href} className={`attn ${a.warn ? 'attn--warn' : ''}`}>
                  <span className="attn__n">{a.n}</span>
                  <span className="attn__l">{a.label}</span>
                  <ArrowRight size={18} weight="bold" className="icon" />
                </Link>
              ))}
            </div>
          ) : (
            <div className="allclear"><CheckCircle size={20} weight="bold" className="icon" />{t('attn.clear')}</div>
          )}

          {editing && (
            <div className="row" style={{ justifyContent: 'space-between' }}>
              <p className="muted" style={{ margin: 0 }}>{t('dash.drag')}</p>
              <Button small variant="ghost" icon={<ArrowCounterClockwise size={15} weight="bold" />} onClick={() => persist(defaults())}>{t('dash.reset')}</Button>
            </div>
          )}
          <div className="widgets">
            {visible.map((w) => {
              const def = CATALOG[w.id]!;
              const controls = (
                <>
                  {(['xs', 'sm', 'md', 'lg'] as const).filter((s) => def.kind === 'panel' ? s !== 'xs' : s !== 'lg').map((s) => <Chip key={s} active={w.size === s} aria-label={`${t('dash.size')} ${t(`dash.size.${s}`)}`} onClick={() => patch(w.id, { size: s })}>{t(`dash.size.${s}`)}</Chip>)}
                  <Button small variant="ghost" aria-label={t('dash.up')} icon={<ArrowUp size={15} weight="bold" />} onClick={() => move(w.id, -1)} />
                  <Button small variant="ghost" aria-label={t('dash.down')} icon={<ArrowDown size={15} weight="bold" />} onClick={() => move(w.id, 1)} />
                  <Button small variant="ghost" aria-label={t('dash.hide')} icon={<EyeSlash size={15} weight="bold" />} onClick={() => patch(w.id, { visible: false })} />
                </>
              );
              return (
                <div
                  key={w.id}
                  className={`widget widget--${w.size} ${overId === w.id ? 'widget--over' : ''} ${dragId === w.id ? 'widget--dragging' : ''}`}
                  data-widget={w.id}
                  draggable={editing}
                  onDragStart={() => setDragId(w.id)}
                  onDragOver={(e) => { if (editing) { e.preventDefault(); setOverId(w.id); } }}
                  onDragLeave={() => setOverId((o) => (o === w.id ? null : o))}
                  onDrop={(e) => { e.preventDefault(); drop(w.id); }}
                  onDragEnd={() => { setDragId(null); setOverId(null); }}
                >
                  {def.kind === 'stat' ? (
                    <>
                      <Tile id={w.id} d={d} />
                      {editing && <div className="widget__bar">{controls}</div>}
                    </>
                  ) : (
                    <Card title={t(def.title)} actions={editing && controls} flush={w.id === 'activity'}>
                      <WidgetBody id={w.id} d={d} />
                    </Card>
                  )}
                </div>
              );
            })}
          </div>
          {editing && hidden.length > 0 && (
            <div className="stack">
              <h2 className="panel__title">{t('dash.hidden')}</h2>
              <div className="row">
                {hidden.map((w) => <Button key={w.id} icon={<Eye size={16} weight="bold" />} onClick={() => reveal(w.id)}>{t('dash.show')} · {t(CATALOG[w.id]!.title)}</Button>)}
              </div>
            </div>
          )}
        </div>
      )}
    </>
  );
}

export default function Page(): ReactNode {
  return (
    <>
      <Dashboard />
    </>
  );
}
