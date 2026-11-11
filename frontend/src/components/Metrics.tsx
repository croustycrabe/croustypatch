'use client';
import { useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { HostMetrics } from '@/lib/types';
import { Card } from './ui';

/** 4096 -> "4 GB" / "4 Go" (binary units, as the machine reports them). */
export function useSize() {
  const { lang } = useI18n();
  return (mb: number): string => {
    const unit = lang === 'fr' ? ['Mo', 'Go', 'To'] : ['MB', 'GB', 'TB'];
    if (mb >= 1024 * 1024) return `${(mb / 1024 / 1024).toFixed(1)} ${unit[2]}`;
    if (mb >= 1024) return `${(mb / 1024).toFixed(mb >= 10 * 1024 ? 0 : 1)} ${unit[1]}`;
    return `${Math.round(mb)} ${unit[0]}`;
  };
}

const tone = (pct: number) => (pct >= 90 ? 'var(--danger)' : pct >= 75 ? 'var(--warning)' : 'var(--success)');

/** A labelled bar: how full something is. */
export function Meter({ label, pct, detail }: { label: string; pct: number | null | undefined; detail?: string }) {
  const v = pct == null ? null : Math.max(0, Math.min(100, pct));
  return (
    <div className="meter">
      <div className="row" style={{ justifyContent: 'space-between', flexWrap: 'nowrap', gap: 8, fontSize: 13 }}>
        <span>{label}</span>
        <span className="muted">{v == null ? '—' : `${Math.round(v)} %`}{detail ? ` · ${detail}` : ''}</span>
      </div>
      <div className="fleetbar" style={{ height: 8, margin: '4px 0 0' }} role="meter" aria-label={label} aria-valuenow={v ?? undefined} aria-valuemin={0} aria-valuemax={100}>
        <span style={{ flex: v ?? 0, background: v == null ? 'transparent' : tone(v) }} />
        <span style={{ flex: 100 - (v ?? 0), background: 'transparent' }} />
      </div>
    </div>
  );
}

/** A small line chart of 0-100 values over time (gaps where the machine did not report). */
export function Spark({ values, label }: { values: (number | null)[]; label: string }) {
  const W = 320, H = 64;
  const pts = values.map((v, i) => (v == null ? null : [(i / Math.max(values.length - 1, 1)) * W, H - (Math.max(0, Math.min(100, v)) / 100) * (H - 4) - 2] as const));
  const segs: (readonly [number, number])[][] = [];
  for (const p of pts) {
    if (!p) { segs.push([]); continue; }
    if (!segs.length) segs.push([]);
    segs[segs.length - 1]!.push(p);
  }
  const last = values.filter((v): v is number => v != null).at(-1);
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} role="img" aria-label={`${label}${last != null ? ` ${Math.round(last)} %` : ''}`} preserveAspectRatio="none" style={{ display: 'block' }}>
      <line x1="0" x2={W} y1={H - 1} y2={H - 1} stroke="var(--border)" />
      <line x1="0" x2={W} y1={H * 0.25} y2={H * 0.25} stroke="var(--border)" strokeDasharray="3 4" />
      {segs.filter((s) => s.length > 1).map((s, i) => <polyline key={i} points={s.map((p) => p.join(',')).join(' ')} fill="none" stroke="var(--primary)" strokeWidth="1.6" vectorEffect="non-scaling-stroke" />)}
      {segs.filter((s) => s.length === 1).map((s, i) => <circle key={`d${i}`} cx={s[0]![0]} cy={s[0]![1]} r="2" fill="var(--primary)" />)}
    </svg>
  );
}

function uptime(secs: number, t: (k: 'time.d' | 'time.h' | 'time.m', v: { n: number }) => string) {
  const d = Math.floor(secs / 86400), h = Math.floor((secs % 86400) / 3600), m = Math.floor((secs % 3600) / 60);
  return d > 0 ? `${t('time.d', { n: d })} ${t('time.h', { n: h })}` : h > 0 ? `${t('time.h', { n: h })} ${t('time.m', { n: m })}` : t('time.m', { n: m });
}

/** Live resource use of one host and the last 24 hours. */
export function HostResources({ hostId }: { hostId: string }) {
  const { t, ago } = useI18n();
  const size = useSize();
  const m = useApi<HostMetrics>(`/hosts/${hostId}/metrics?hours=24`, { poll: 15000 });
  if (!m.data) return null;
  const c = m.data.current;
  const has = m.data.at != null && c.mem_total_mb != null;
  const rows = m.data.points;
  return (
    <Card title={t('host.res.title')} actions={m.data.at ? <span className="muted" style={{ fontSize: 12 }}>{t('host.res.updated', { when: ago(m.data.at) })}</span> : undefined}>
      {!has ? <p className="muted">{t('host.res.none')}</p> : (
        <div className="stack">
          <Meter label={t('host.res.cpu')} pct={c.cpu_pct} detail={c.load_1m != null ? `${t('host.res.load')} ${c.load_1m.toFixed(2)} · ${c.load_5m?.toFixed(2)} · ${c.load_15m?.toFixed(2)}` : undefined} />
          <Meter label={t('host.res.ram')} pct={c.mem_pct} detail={`${size(c.mem_used_mb ?? 0)} / ${size(c.mem_total_mb ?? 0)}`} />
          {(c.swap_total_mb ?? 0) > 0 && <Meter label={t('host.res.swap')} pct={((c.swap_used_mb ?? 0) / (c.swap_total_mb ?? 1)) * 100} detail={`${size(c.swap_used_mb ?? 0)} / ${size(c.swap_total_mb ?? 0)}`} />}
          {(c.disks ?? []).map((d) => <Meter key={d.mount} label={`${t('host.res.disk')} ${d.mount}`} pct={d.total_mb > 0 ? (d.used_mb / d.total_mb) * 100 : null} detail={`${size(d.used_mb)} / ${size(d.total_mb)} · ${d.fs}`} />)}
          {c.uptime_secs != null && <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t('host.res.uptime')} : {uptime(c.uptime_secs, t)}</p>}
          {rows.length > 1 && (
            <div className="grid grid--2" style={{ gap: 16 }}>
              <div><p className="label" style={{ marginBottom: 4 }}>{t('host.res.cpu')} · 24 h</p><Spark values={rows.map((p) => p.cpu)} label={t('host.res.cpu')} /></div>
              <div><p className="label" style={{ marginBottom: 4 }}>{t('host.res.ram')} · 24 h</p><Spark values={rows.map((p) => p.mem)} label={t('host.res.ram')} /></div>
            </div>
          )}
        </div>
      )}
    </Card>
  );
}
