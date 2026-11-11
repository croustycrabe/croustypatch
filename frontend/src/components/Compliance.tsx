'use client';
import { PencilSimple, Plus, ShieldCheck, Trash, X } from '@phosphor-icons/react';
import Link from 'next/link';
import { useEffect, useState } from 'react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { useToast } from '@/lib/toast';
import type { CompFleet, CompHost, CompProfile, CompProfiles, FailingRule, Settings } from '@/lib/types';
import { Button, Card, ConfirmDialog, Dialog, Empty, Field, Notice, Select, Spinner, StatCard, Switch, Table, Tag, TextArea, TextInput } from './ui';

const RULE_PREFIX = 'xccdf_org.ssgproject.content_rule_';
const CUSTOM = 'custom:';
/** A rule id without the prefix every SSG rule shares. */
export const shortRule = (id: string) => id.replace(RULE_PREFIX, '');
const sevTone = (s: string) => (s === 'high' ? 'danger' : s === 'medium' ? 'warn' : 'neutral') as 'danger' | 'warn' | 'neutral';
const scoreTone = (score: number, threshold: number) => (score >= threshold ? 'ok' : score >= threshold - 15 ? 'warn' : 'danger') as 'ok' | 'warn' | 'danger';

/** "CIS Debian Linux 12 …" for a profile id, the custom profile's name, or "fleet default". */
export function useProfileLabel(profiles: CompProfiles | undefined) {
  const { t } = useI18n();
  return (ref: string): string => {
    if (!ref) return t('comp.profile.default');
    if (ref.startsWith(CUSTOM)) return profiles?.custom.find((p) => p.id === ref.slice(CUSTOM.length))?.name ?? t('comp.profile.gone');
    return profiles?.available.find((p) => p.id === ref)?.title ?? ref.replace('xccdf_org.ssgproject.content_profile_', '');
  };
}

/** Profile chooser: the fleet default, the profiles of the SCAP content, and the custom ones. */
export function ProfileSelect({ id, value, onChange, profiles, available, emptyLabel, disabled }: { id?: string; value: string; onChange: (v: string) => void; profiles: CompProfiles; available?: { id: string; title: string }[]; emptyLabel: string; disabled?: boolean }) {
  const { t } = useI18n();
  const std = available?.length ? available : profiles.available;
  const known = value === '' || value.startsWith(CUSTOM) || std.some((p) => p.id === value);
  return (
    <Select id={id} value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
      <option value="">{emptyLabel}</option>
      {std.length > 0 && (
        <optgroup label={t('comp.profile.group.std')}>
          {std.map((p) => <option key={p.id} value={p.id}>{p.title || shortRule(p.id)}</option>)}
          {!known && <option value={value}>{value}</option>}
        </optgroup>
      )}
      {!std.length && !known && <option value={value}>{value}</option>}
      {profiles.custom.length > 0 && (
        <optgroup label={t('comp.profile.group.custom')}>
          {profiles.custom.map((p) => <option key={p.id} value={`${CUSTOM}${p.id}`}>{p.name}</option>)}
        </optgroup>
      )}
    </Select>
  );
}

/** The Compliance tab of a host: is OpenSCAP there, which profile, the last audits. */
export function HostCompliance({ hostId, modules, canScan, onScan }: { hostId: string; modules: string[]; canScan: boolean; onScan: () => void }) {
  const { t, date } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const act = useAction();
  const r = useApi<CompHost>(`/hosts/${hostId}/compliance`, { poll: 15000 });
  const profiles = useApi<CompProfiles>('/compliance/profiles');
  const last = r.data?.reports[0];
  const info = r.data?.openscap ?? null;
  const label = useProfileLabel(profiles.data);

  const setProfile = (v: string) => act.run(async () => {
    await api('PATCH', `/hosts/${hostId}`, { compliance_profile: v });
    toast.success(t('common.saved'));
    r.reload();
  });

  return (
    <div className="stack stack--lg">
      {act.error && <Notice kind="error">{act.error}</Notice>}
      <div className="row" style={{ justifyContent: 'space-between' }}>
        <span className="muted">{t('comp.modules')}: {modules.length ? modules.map((m) => <Tag key={m}>{m}</Tag>) : '—'}</span>
        {canScan && <Button icon={<ShieldCheck size={16} weight="bold" />} onClick={onScan}>{t('comp.scan')}</Button>}
      </div>

      {r.data && (
        <Card title={t('comp.status')}>
          <div className="stack">
            {!r.data.module ? <Notice kind="info">{t('comp.status.off')}</Notice>
              : !info ? <Notice kind="info">{t('comp.status.missing')} {r.data.auto_install ? t('comp.status.missing.auto') : t('comp.status.missing.manual')}</Notice>
              : !info.datastream ? <Notice kind="info">{t('comp.status.nocontent', { version: info.version })}</Notice>
              : <p style={{ margin: 0 }}>{t('comp.status.ok', { version: info.version, n: info.profiles.length })} <span className="muted mono" style={{ fontSize: 12 }}>{info.datastream.split('/').pop()}</span></p>}
            {profiles.data && (
              <Field label={t('comp.profile.host')} hint={r.data.profile === '' ? t('comp.profile.effective', { name: label(r.data.effective_profile) }) : undefined}>
                {(i) => <ProfileSelect id={i} value={r.data!.profile} profiles={profiles.data!} available={info?.profiles} emptyLabel={t('comp.profile.default')} disabled={!can('hosts:write') || act.pending} onChange={(v) => void setProfile(v)} />}
              </Field>
            )}
          </div>
        </Card>
      )}

      {!r.data ? <Spinner /> : !last ? <Empty>{t('comp.none')}</Empty> : (
        <>
          <div className="grid grid--stats" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(10rem, 1fr))' }}>
            <StatCard value={`${last.score}%`} label={t('comp.score')} tone={last.score >= 90 ? 'ok' : last.score >= 70 ? 'warn' : 'danger'} />
            <StatCard value={last.passed} label={t('comp.passed')} />
            <StatCard value={last.failed} label={t('comp.failed')} tone={last.failed ? 'warn' : 'ok'} />
            <StatCard value={last.not_applicable} label={t('comp.na')} />
          </div>
          <dl className="kv">
            <dt>{t('comp.profile')}</dt><dd className="mono">{last.profile}</dd>
            <dt>{t('hosts.col.seen')}</dt><dd>{date(last.collected_at)}</dd>
            <dt>{t('comp.hash')}</dt><dd className="mono" style={{ wordBreak: 'break-all' }}>{last.results_sha256}</dd>
          </dl>
          {last.failed_rules && last.failed_rules.length > 0 && (
            <Table head={[t('comp.failed_rules'), t('comp.severity')]}>
              {last.failed_rules.map((f) => (
                <tr key={f.rule_id}><td className="mono wrap">{shortRule(f.rule_id)}</td><td><Tag tone={sevTone(f.severity)}>{f.severity}</Tag></td></tr>
              ))}
            </Table>
          )}
          {r.data.reports.length > 1 && (
            <Table head={[t('comp.history'), t('comp.score'), t('comp.failed')]}>
              {r.data.reports.slice(1).map((x) => <tr key={x.id}><td>{date(x.collected_at)}</td><td className="num">{x.score}%</td><td className="num">{x.failed}</td></tr>)}
            </Table>
          )}
        </>
      )}
    </div>
  );
}

/** Create or change a custom profile: a base profile of the SCAP content and the rules switched off. */
function ProfileEditor({ profile, profiles, onClose, onSaved }: { profile: CompProfile | 'new' | null; profiles: CompProfiles; onClose: () => void; onSaved: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const act = useAction();
  const existing = profile && profile !== 'new' ? profile : null;
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [base, setBase] = useState('');
  const [off, setOff] = useState<string[]>([]);
  const [filter, setFilter] = useState('');
  const [typed, setTyped] = useState('');
  const [typedErr, setTypedErr] = useState('');
  const rules = useApi<{ rules: FailingRule[] }>(profile ? '/compliance/rules' : null);

  useEffect(() => {
    if (!profile) return;
    setName(existing?.name ?? '');
    setDescription(existing?.description ?? '');
    setBase(existing?.base_profile ?? profiles.available.find((p) => p.id.endsWith('_profile_standard'))?.id ?? profiles.available[0]?.id ?? '');
    setOff(existing?.disabled_rules ?? []);
    setFilter(''); setTyped(''); setTypedErr('');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profile]);

  const add = (id: string) => setOff((o) => (o.includes(id) ? o : [...o, id].sort()));
  const addTyped = () => {
    const v = typed.trim();
    if (!v) return;
    const id = v.startsWith('xccdf_') ? v : `${RULE_PREFIX}${v}`;
    if (!/^[A-Za-z0-9_.-]{1,300}$/.test(id)) { setTypedErr(t('comp.editor.badrule')); return; }
    add(id); setTyped(''); setTypedErr('');
  };
  const suggestions = (rules.data?.rules ?? []).filter((r) => !off.includes(r.rule_id) && shortRule(r.rule_id).toLowerCase().includes(filter.trim().toLowerCase())).slice(0, 40);

  const save = () => act.run(async () => {
    const body = { name, description, base_profile: base, disabled_rules: off };
    if (existing) await api('PUT', `/compliance/profiles/${existing.id}`, body);
    else await api('POST', '/compliance/profiles', body);
    toast.success(t('common.saved'));
    onSaved();
    onClose();
  });

  return (
    <Dialog open={!!profile} title={existing ? t('comp.editor.edit') : t('comp.editor.new')} onClose={onClose} wide>
      <form className="stack" onSubmit={(e) => { e.preventDefault(); void save(); }}>
        {act.error && <Notice kind="error">{act.error}</Notice>}
        <Field label={t('comp.profile.name')}>{(i) => <TextInput id={i} value={name} maxLength={80} required onChange={(e) => setName(e.target.value)} />}</Field>
        <Field label={t('comp.profile.description')}>{(i) => <TextArea id={i} rows={2} value={description} maxLength={500} onChange={(e) => setDescription(e.target.value)} />}</Field>
        <Field label={t('comp.profile.base')} hint={t('comp.profile.base.hint')}>
          {(i) => profiles.available.length > 0 || base
            ? (
              <Select id={i} value={base} onChange={(e) => setBase(e.target.value)} required>
                {profiles.available.map((p) => <option key={p.id} value={p.id}>{p.title || shortRule(p.id)}</option>)}
                {base && !profiles.available.some((p) => p.id === base) && <option value={base}>{base}</option>}
              </Select>
            )
            : <TextInput id={i} className="mono" value={base} placeholder="xccdf_org.ssgproject.content_profile_standard" required onChange={(e) => setBase(e.target.value)} />}
        </Field>

        <div>
          <p className="label">{t('comp.editor.off', { n: off.length })}</p>
          {off.length === 0 ? <p className="muted" style={{ margin: 0 }}>{t('comp.editor.none')}</p> : (
            <div className="row row--tight" role="list" aria-label={t('comp.editor.off', { n: off.length })}>
              {off.map((r) => (
                <span key={r} className="chip chip--rule" role="listitem" title={r}>
                  <span className="mono" style={{ fontSize: 12 }}>{shortRule(r)}</span>
                  <button type="button" className="chip__x" aria-label={t('comp.editor.keep', { rule: shortRule(r) })} onClick={() => setOff(off.filter((x) => x !== r))}><X size={12} weight="bold" /></button>
                </span>
              ))}
            </div>
          )}
        </div>

        <div className="stack" style={{ gap: 8 }}>
          <p className="label" style={{ margin: 0 }}>{t('comp.editor.suggest')}</p>
          <TextInput aria-label={t('comp.editor.filter')} placeholder={t('comp.editor.filter')} value={filter} onChange={(e) => setFilter(e.target.value)} />
          {!rules.data ? <Spinner /> : suggestions.length === 0 ? <p className="muted" style={{ margin: 0 }}>{t('comp.editor.suggest.none')}</p> : (
            <div className="row row--tight">
              {suggestions.map((r) => (
                <button type="button" key={r.rule_id} className="chip" title={`${r.rule_id} · ${t('comp.editor.on_hosts', { n: r.hosts })}`} onClick={() => add(r.rule_id)}>
                  <span className="mono" style={{ fontSize: 12 }}>{shortRule(r.rule_id)}</span> <Tag tone={sevTone(r.severity)}>{r.severity}</Tag>
                </button>
              ))}
            </div>
          )}
        </div>

        <Field label={t('comp.editor.add')} hint={typedErr || t('comp.editor.add.hint')}>
          {(i) => (
            <div className="row" style={{ flexWrap: 'nowrap' }}>
              <TextInput id={i} className="mono" value={typed} placeholder="package_aide_installed" onChange={(e) => { setTyped(e.target.value); setTypedErr(''); }} onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addTyped(); } }} />
              <Button type="button" onClick={addTyped}>{t('comp.editor.add.go')}</Button>
            </div>
          )}
        </Field>

        <div className="row">
          <Button variant="primary" type="submit" disabled={act.pending || !name.trim() || !base}>{t('common.save')}</Button>
          <Button type="button" onClick={onClose}>{t('common.cancel')}</Button>
        </div>
      </form>
    </Dialog>
  );
}

/** The Compliance page: fleet scores, audit settings and the custom profiles. */
export function CompliancePage() {
  const { t, date } = useI18n();
  const { can } = useAuth();
  const toast = useToast();
  const act = useAction();
  const fleet = useApi<CompFleet>('/compliance', { poll: 30000 });
  const profiles = useApi<CompProfiles>('/compliance/profiles');
  const settings = useApi<Settings>(can('settings:read') ? '/settings' : null);
  const [edit, setEdit] = useState<CompProfile | 'new' | null>(null);
  const [del, setDel] = useState<CompProfile | null>(null);
  const [cfg, setCfg] = useState<Settings['compliance'] | null>(null);
  const writable = can('settings:write');
  const label = useProfileLabel(profiles.data);

  useEffect(() => { if (settings.data) setCfg(settings.data.compliance); }, [settings.data]);

  if (!fleet.data || !profiles.data) return fleet.error || profiles.error ? <Notice kind="error">{(fleet.error ?? profiles.error)!.message}</Notice> : <Spinner />;
  const f = fleet.data, p = profiles.data;

  const saveCfg = () => act.run(async () => {
    if (!cfg) return;
    await api('PUT', '/settings/compliance', cfg);
    toast.success(t('common.saved'));
    settings.reload(); profiles.reload(); fleet.reload();
  });
  const remove = (x: CompProfile) => act.run(async () => {
    await api('DELETE', `/compliance/profiles/${x.id}`);
    setDel(null);
    toast.success(t('comp.profiles.deleted'));
    profiles.reload();
  });

  return (
    <div className="stack stack--lg">
      {act.error && <Notice kind="error">{act.error}</Notice>}
      <div className="grid grid--stats">
        <StatCard value={f.score == null ? '—' : `${f.score}%`} label={t('comp.stat.score')} tone={f.score == null ? undefined : scoreTone(f.score, f.threshold)} />
        <StatCard value={f.hosts.length} label={t('comp.stat.audited')} />
        <StatCard value={p.hosts_with_openscap} label={t('comp.stat.capable')} />
        <StatCard value={`${f.threshold}%`} label={t('comp.stat.threshold')} />
      </div>

      <Card title={t('comp.hosts')} flush>
        <Table head={[t('comp.col.host'), t('comp.col.profile'), t('comp.col.score'), t('comp.col.failed'), t('comp.col.date')]} empty={f.hosts.length === 0 ? <Empty>{t('comp.hosts.empty')}</Empty> : undefined}>
          {f.hosts.map((h) => (
            <tr key={h.id}>
              <td><Link href={`/host?id=${h.host_id}&tab=compliance`}>{h.hostname ?? h.host_id}</Link></td>
              <td>{h.profile}</td>
              <td><Tag tone={scoreTone(h.score, f.threshold)}>{h.score}%</Tag></td>
              <td className="num">{h.failed}</td>
              <td>{date(h.collected_at)}</td>
            </tr>
          ))}
        </Table>
      </Card>

      {cfg && (
        <Card title={t('comp.settings')} actions={writable ? <Button variant="primary" small disabled={act.pending} onClick={() => void saveCfg()}>{t('common.save')}</Button> : undefined}>
          <div className="stack">
            <Field label={t('comp.settings.default')} hint={t('comp.settings.default.hint')}>
              {(i) => <ProfileSelect id={i} value={cfg.profile} profiles={p} emptyLabel={t('comp.settings.default.none')} disabled={!writable} onChange={(v) => setCfg({ ...cfg, profile: v })} />}
            </Field>
            <Field label={t('settings.compliance.threshold')}>
              {(i) => <TextInput id={i} type="number" min={0} max={100} value={cfg.threshold} disabled={!writable} onChange={(e) => setCfg({ ...cfg, threshold: Number(e.target.value) })} />}
            </Field>
            <div className="row" style={{ justifyContent: 'space-between', flexWrap: 'nowrap' }}>
              <div>
                <b>{t('comp.settings.auto')}</b>
                <p className="field__hint">{t('comp.settings.auto.hint')}</p>
              </div>
              <Switch checked={cfg.auto_install} disabled={!writable} onChange={(v) => setCfg({ ...cfg, auto_install: v })}><span className="sr-only">{t('comp.settings.auto')}</span></Switch>
            </div>
          </div>
        </Card>
      )}

      <Card title={t('comp.profiles')} actions={writable ? <Button small icon={<Plus size={14} weight="bold" />} onClick={() => setEdit('new')}>{t('comp.profiles.new')}</Button> : undefined}>
        <p className="muted" style={{ marginTop: 0 }}>{t('comp.profiles.hint')}</p>
        <Table head={[t('comp.profile.name'), t('comp.profile.base'), t('comp.profiles.off'), t('comp.profiles.used'), '']} empty={p.custom.length === 0 ? <Empty>{t('comp.profiles.none')}</Empty> : undefined}>
          {p.custom.map((x) => (
            <tr key={x.id}>
              <td><b>{x.name}</b>{x.description && <div className="muted" style={{ fontSize: 12 }}>{x.description}</div>}</td>
              <td>{label(x.base_profile)}</td>
              <td className="num">{x.disabled_rules.length}</td>
              <td className="num">{x.used_by}</td>
              <td>
                {writable && (
                  <span className="row row--tight right">
                    <Button small icon={<PencilSimple size={14} />} aria-label={`${t('common.edit')} ${x.name}`} onClick={() => setEdit(x)}>{t('common.edit')}</Button>
                    <Button small variant="danger" icon={<Trash size={14} />} aria-label={`${t('common.delete')} ${x.name}`} onClick={() => setDel(x)}>{t('common.delete')}</Button>
                  </span>
                )}
              </td>
            </tr>
          ))}
        </Table>
      </Card>

      <ProfileEditor profile={edit} profiles={p} onClose={() => setEdit(null)} onSaved={() => { profiles.reload(); fleet.reload(); }} />
      <ConfirmDialog danger message={del ? t('comp.profiles.delete', { name: del.name }) : null} onNo={() => setDel(null)} onYes={() => { if (del) void remove(del); }} />
    </div>
  );
}
