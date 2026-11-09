'use client';
import { ArrowDown, ArrowRight, ArrowUp, CheckCircle, Eye, EyeSlash, SlidersHorizontal } from '@phosphor-icons/react';
import Link from 'next/link';
import { useEffect, useState, type ReactNode } from 'react';
import { AddAgentButton } from '@/components/AddAgent';
import { Button, Card, Chip, Empty, Notice, PageHeader, Spinner, StatCard, StatusDot, Tag, TaskStatusLabel, HOST_DOT, HOST_TONE } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import type { Dashboard, HostStatus, Layout } from '@/lib/types';

const IDS = ['fleet', 'os', 'packages', 'vulns', 'activity', 'portal'] as const;
type WidgetId = (typeof IDS)[number];
type Size = 'sm' | 'md' | 'lg';
type Widget = { id: string; visible: boolean; size: Size };
const TITLES: Record<WidgetId, MessageKey> = { fleet: 'w.fleet', os: 'w.os', packages: 'w.packages', vulns: 'w.vulns', activity: 'w.activity', portal: 'w.portal' };
const DEFAULT_SIZE: Record<WidgetId, Size> = { fleet: 'md', os: 'md', packages: 'md', vulns: 'md', activity: 'lg', portal: 'md' };

// Segment colours come from the palette: ink for healthy, brick for trouble, tints in between.
const BAR: Record<HostStatus, string> = {
  online: 'var(--success)', eco: 'var(--info)', maintenance: 'var(--warning)', offline: 'var(--border-strong)', alert: 'var(--danger)', quarantined: 'var(--danger)', pending: 'var(--surface-active)',
};

/** Keep only known widgets, in the saved order, and append any widget the saved layout predates. */
function normalise(saved: Layout | undefined): Widget[] {
  const known = (saved?.widgets ?? []).filter((w): w is Widget => (IDS as readonly string[]).includes(w.id));
  const missing = IDS.filter((id) => !known.some((w) => w.id === id)).map((id) => ({ id, visible: true, size: DEFAULT_SIZE[id] }));
  return [...known, ...missing];
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

function WidgetBody({ id, d }: { id: WidgetId; d: Dashboard }) {
  const { t, tr, ago } = useI18n();
  const total = Math.max(d.fleet.total, 1);
  switch (id) {
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
          <StatCard value={d.compliance.enabled && d.compliance.score !== null ? `${d.compliance.score}%` : '—'} label={t('w.vulns.score')} />
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
          <div className="grid grid--stats" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(11rem, 1fr))' }}>
            <StatCard value={d.fleet.total} label={t('dash.cards.hosts')} href="/hosts" />
            <StatCard value={d.fleet.hosts_needing_updates} tone={d.fleet.hosts_needing_updates > 0 ? 'warn' : 'ok'} label={t('dash.cards.needing')} href="/hosts?updates=1" />
            <StatCard value={d.vulnerabilities.security_updates} label={t('dash.cards.security')} alert={d.vulnerabilities.security_updates > 0} href="/hosts?security=1" />
            <StatCard value={d.vulnerabilities.outdated_packages} tone={d.vulnerabilities.outdated_packages > 0 ? 'warn' : 'ok'} label={t('dash.cards.outdated')} href="/packages?updates=1" />
            <StatCard value={attentionHosts} label={t('dash.cards.attention')} alert={attentionHosts > 0} href="/hosts?attention=1" />
          </div>

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

          {editing && <p className="muted">{t('dash.drag')}</p>}
          <div className="widgets">
            {visible.map((w) => (
              <div
                key={w.id}
                className={`widget widget--${w.size} ${overId === w.id ? 'widget--over' : ''} ${dragId === w.id ? 'widget--dragging' : ''}`}
                draggable={editing}
                onDragStart={() => setDragId(w.id)}
                onDragOver={(e) => { if (editing) { e.preventDefault(); setOverId(w.id); } }}
                onDragLeave={() => setOverId((o) => (o === w.id ? null : o))}
                onDrop={(e) => { e.preventDefault(); drop(w.id); }}
                onDragEnd={() => { setDragId(null); setOverId(null); }}
              >
                <Card
                  title={t(TITLES[w.id as WidgetId])}
                  actions={editing && (
                    <>
                      {(['sm', 'md', 'lg'] as const).map((s) => <Chip key={s} active={w.size === s} aria-label={`${t('dash.size')} ${t(`dash.size.${s}`)}`} onClick={() => patch(w.id, { size: s })}>{t(`dash.size.${s}`)}</Chip>)}
                      <Button small variant="ghost" aria-label={t('dash.up')} icon={<ArrowUp size={15} weight="bold" />} onClick={() => move(w.id, -1)} />
                      <Button small variant="ghost" aria-label={t('dash.down')} icon={<ArrowDown size={15} weight="bold" />} onClick={() => move(w.id, 1)} />
                      <Button small variant="ghost" aria-label={t('dash.hide')} icon={<EyeSlash size={15} weight="bold" />} onClick={() => patch(w.id, { visible: false })} />
                    </>
                  )}
                  flush={w.id === 'activity'}
                >
                  <WidgetBody id={w.id as WidgetId} d={d} />
                </Card>
              </div>
            ))}
          </div>
          {editing && hidden.length > 0 && (
            <div className="stack">
              <h2 className="panel__title">{t('dash.hidden')}</h2>
              <div className="row">
                {hidden.map((w) => <Button key={w.id} icon={<Eye size={16} weight="bold" />} onClick={() => patch(w.id, { visible: true })}>{t('dash.show')} · {t(TITLES[w.id as WidgetId])}</Button>)}
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
