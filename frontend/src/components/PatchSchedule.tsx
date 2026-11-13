'use client';
import { useEffect, useMemo, useState } from 'react';
import { api } from '@/lib/api';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import type { MaintenanceWindow, Occurrence, PolicyDef } from '@/lib/types';
import { Chip, Field, Notice, Select, TextInput } from './ui';

/** A zone the browser can format with; anything else (a typo being typed, a zone newer than this browser) is shown in UTC
 * instead of throwing while the page renders. */
export function safeZone(z?: string | null): string {
  if (!z) return 'UTC';
  try {
    new Intl.DateTimeFormat('en', { timeZone: z });
    return z;
  } catch {
    return 'UTC';
  }
}

export const browserZone = (): string => {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
};

export const newWindow = (over: Partial<MaintenanceWindow> = {}): MaintenanceWindow => ({
  days: [5], start: '02:00', end: '05:00', utc_offset_minutes: 0, timezone: browserZone(), repeat: 'weekly', nth: null, day_of_month: null, ...over,
});

/** "every Saturday", "the first Monday of each month", "the 15th of each month". */
export function useWindowText() {
  const { t, tr } = useI18n();
  return (w: MaintenanceWindow): string => {
    const days = [...w.days].sort();
    const names = days.map((d) => tr('dayfull', String(d)));
    if (w.repeat === 'monthly_weekday') return t('pol.when.monthly_weekday', { nth: tr('nth', String(w.nth ?? 1)), day: names[0] ?? '' });
    if (w.repeat === 'monthly_day') return t('pol.when.monthly_day', { day: w.day_of_month ?? 1 });
    if (days.length === 0 || days.length === 7) return t('pol.when.every_day');
    return t('pol.when.weekly', { days: names.join(', ') });
  };
}

/** Next patch days, in the time zone of the window. */
export function Occurrences({ list, zone }: { list: Occurrence[]; zone?: string | null }) {
  const { date } = useI18n();
  const tz = safeZone(zone);
  const day = (iso: string) => date(iso, { timeZone: tz, weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
  const hm = (iso: string) => date(iso, { timeZone: tz, hour: '2-digit', minute: '2-digit' });
  return (
    <ul style={{ margin: 0, paddingLeft: 20, fontSize: 13 }}>
      {list.map((o) => <li key={o.start}>{day(o.start)} · {hm(o.start)} – {hm(o.end)}</li>)}
    </ul>
  );
}

const PRESETS: { key: MessageKey; make: () => MaintenanceWindow }[] = [
  { key: 'pol.preset.saturday', make: () => newWindow({ days: [5] }) },
  { key: 'pol.preset.first_monday', make: () => newWindow({ days: [0], repeat: 'monthly_weekday', nth: 1 }) },
  { key: 'pol.preset.mid_month', make: () => newWindow({ days: [], repeat: 'monthly_day', day_of_month: 15 }) },
];

/** The "when are updates applied" part of a policy: approval or automatic, and the patch day. */
export function PatchWhen({ d, edit }: { d: PolicyDef; edit: (fn: (n: PolicyDef) => void) => void }) {
  const { t, tr } = useI18n();
  const text = useWindowText();
  const w = d.maintenance_window;
  const auto = d.default_action === 'AUTO_DEPLOY';
  const setW = (fn: (n: MaintenanceWindow) => void) => edit((n) => { if (n.maintenance_window) fn(n.maintenance_window); });
  const [prev, setPrev] = useState<{ valid: boolean; error: string | null; next: Occurrence[]; open?: boolean } | null>(null);
  const wKey = JSON.stringify(w);
  useEffect(() => {
    if (!w) { setPrev(null); return; }
    let live = true;
    const h = setTimeout(() => { api<typeof prev>('POST', '/policies/preview-schedule', { window: w }).then((r) => { if (live) setPrev(r); }, () => { if (live) setPrev(null); }); }, 350);
    return () => { live = false; clearTimeout(h); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wKey]);
  const zones = useMemo(() => {
    try {
      return (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.('timeZone') ?? [];
    } catch {
      return [];
    }
  }, []);
  const times = w ? `${w.start} – ${w.end} (${w.timezone || `UTC${w.utc_offset_minutes >= 0 ? '+' : ''}${w.utc_offset_minutes / 60}`})` : '';

  return (
    <div className="stack">
      <fieldset className="fieldset">
        <legend>{t('pol.when.what')}</legend>
        <div className="chips">
          <Chip active={!auto} onClick={() => edit((n) => { n.default_action = 'REQUIRE_APPROVAL'; })}>{t('pol.when.approval')}</Chip>
          <Chip active={auto} onClick={() => edit((n) => { n.default_action = 'AUTO_DEPLOY'; })}>{t('pol.when.auto')}</Chip>
        </div>
        {d.default_action === 'BLOCK' && <p className="field__hint" style={{ marginTop: 8 }}>{t('pol.when.block')}</p>}
        {d.decisions.length > 0 && <p className="field__hint" style={{ marginTop: 8 }}>{t('pol.when.decisions')}</p>}
      </fieldset>

      <fieldset className="fieldset">
        <legend>{t('pol.when.when')}</legend>
        <div className="chips">
          <Chip active={!w} onClick={() => edit((n) => { n.maintenance_window = null; })}>{t('pol.when.anytime')}</Chip>
          <Chip active={!!w} onClick={() => edit((n) => { if (!n.maintenance_window) n.maintenance_window = newWindow(); })}>{t('pol.when.patchday')}</Chip>
        </div>
        {w && (
          <div className="stack" style={{ marginTop: 14 }}>
            <div className="chips">
              {PRESETS.map((p) => <Chip key={p.key} active={false} onClick={() => edit((n) => { n.maintenance_window = { ...p.make(), start: n.maintenance_window?.start ?? '02:00', end: n.maintenance_window?.end ?? '05:00', timezone: n.maintenance_window?.timezone ?? browserZone() }; })}>{t(p.key)}</Chip>)}
            </div>
            <div className="form-grid">
              <Field label={t('pol.patchday.repeat')}>{(id) => (
                <Select id={id} value={w.repeat ?? 'weekly'} onChange={(e) => setW((x) => {
                  const r = e.target.value as NonNullable<MaintenanceWindow['repeat']>;
                  x.repeat = r;
                  if (r === 'monthly_weekday') { x.nth = x.nth ?? 1; x.days = x.days.length ? [x.days[0]!] : [0]; }
                  if (r === 'monthly_day') x.day_of_month = x.day_of_month ?? 1;
                })}>
                  <option value="weekly">{t('pol.patchday.weekly')}</option>
                  <option value="monthly_weekday">{t('pol.patchday.monthly_weekday')}</option>
                  <option value="monthly_day">{t('pol.patchday.monthly_day')}</option>
                </Select>)}</Field>
              {w.repeat === 'monthly_weekday' && (
                <>
                  <Field label={t('pol.patchday.nth')}>{(id) => <Select id={id} value={w.nth ?? 1} onChange={(e) => setW((x) => { x.nth = Number(e.target.value); })}>{[1, 2, 3, 4, 5].map((n) => <option key={n} value={n}>{tr('nth', String(n))}</option>)}</Select>}</Field>
                  <Field label={t('pol.patchday.weekday')}>{(id) => <Select id={id} value={w.days[0] ?? 0} onChange={(e) => setW((x) => { x.days = [Number(e.target.value)]; })}>{[0, 1, 2, 3, 4, 5, 6].map((n) => <option key={n} value={n}>{tr('dayfull', String(n))}</option>)}</Select>}</Field>
                </>
              )}
              {w.repeat === 'monthly_day' && <Field label={t('pol.patchday.day_of_month')}>{(id) => <TextInput id={id} type="number" min={1} max={28} value={w.day_of_month ?? 1} onChange={(e) => setW((x) => { x.day_of_month = Number(e.target.value); })} />}</Field>}
            </div>
            {(w.repeat ?? 'weekly') === 'weekly' && (
              <div>
                <div className="chips">
                  {[0, 1, 2, 3, 4, 5, 6].map((day) => <Chip key={day} active={w.days.includes(day)} onClick={() => setW((x) => { x.days = x.days.includes(day) ? x.days.filter((y) => y !== day) : [...x.days, day].sort(); })}>{tr('day', String(day))}</Chip>)}
                </div>
                {w.days.length === 0 && <p className="field__hint" style={{ marginTop: 8 }}>{t('pol.window.every_day')}</p>}
              </div>
            )}
            <div className="form-grid">
              <Field label={t('pol.window.start')}>{(id) => <TextInput id={id} type="time" value={w.start} onChange={(e) => setW((x) => { x.start = e.target.value; })} />}</Field>
              <Field label={t('pol.window.end')}>{(id) => <TextInput id={id} type="time" value={w.end} onChange={(e) => setW((x) => { x.end = e.target.value; })} />}</Field>
              <Field label={t('pol.patchday.timezone')} hint={t('pol.patchday.timezone_hint')}>{(id) => (
                <>
                  <TextInput id={id} list="cp-zones" value={w.timezone ?? ''} placeholder="Europe/Paris" onChange={(e) => setW((x) => { x.timezone = e.target.value || null; })} />
                  <datalist id="cp-zones">{zones.map((z) => <option key={z} value={z} />)}</datalist>
                </>)}</Field>
            </div>
            {prev && !prev.valid && <Notice kind="error">{prev.error}</Notice>}
            {prev?.valid && (
              <div aria-live="polite">
                <p className="label" style={{ marginBottom: 6 }}>{t('pol.patchday.next')}{prev.open ? ` · ${t('pol.in_window')}` : ''}</p>
                <Occurrences list={prev.next} zone={w.timezone} />
              </div>
            )}
          </div>
        )}
      </fieldset>

      <Notice kind={auto ? 'info' : 'ok'}>
        {auto
          ? (w ? t('pol.summary.auto_day', { when: text(w), times }) : t('pol.summary.auto_now'))
          : (w ? t('pol.summary.approval_day', { when: text(w), times }) : t('pol.summary.approval'))}
      </Notice>
    </div>
  );
}
