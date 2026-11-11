'use client';
import { useState } from 'react';
import { Button, Check, Chip, ConfirmDialog, Drawer, Empty, Field, GroupTag, Notice, PageHeader, Section, Select, Spinner, Table, Tag, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction, useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { PatchWhen, safeZone, useWindowText } from '@/components/PatchSchedule';
import type { Assessment, HostGroup, HostSummary, Policy, PolicyAction, PolicyDef, Probe, Risk } from '@/lib/types';

const RISKS: Risk[] = ['Low', 'Medium', 'High', 'Critical'];
const ACTIONS: PolicyAction[] = ['AUTO_DEPLOY', 'REQUIRE_APPROVAL', 'BLOCK'];

const blankDef = (): PolicyDef => ({
  ai_mode_enabled: false, target_scope: '', security_only: true, rules: [], decisions: [], default_action: 'REQUIRE_APPROVAL', maintenance_window: null,
  rollback: { soak_minutes: 30, probes: [], auto_rollback: true, snapshot_required: true }, reboot: { mode: '' }, rings: { delay_minutes: 0 },
});

interface Form { id?: string; name: string; description: string; enabled: boolean; priority: number; group_ids: string[]; def: PolicyDef }

function RiskSelect({ id, value, onChange, any }: { id?: string; value: Risk | null | undefined; onChange: (v: Risk | null) => void; any?: boolean }) {
  const { t, tr } = useI18n();
  return (
    <Select id={id} value={value ?? ''} onChange={(e) => onChange((e.target.value || null) as Risk | null)}>
      {any && <option value="">{t('pol.decision.any')}</option>}
      {RISKS.map((r) => <option key={r} value={r}>{tr('risk', r)}</option>)}
    </Select>
  );
}

function Editor({ form, setForm, groups, hosts }: { form: Form; setForm: (f: Form) => void; groups: HostGroup[]; hosts: HostSummary[] }) {
  const { t, tr } = useI18n();
  const d = form.def;
  const edit = (fn: (n: PolicyDef) => void) => { const n = structuredClone(d); fn(n); setForm({ ...form, def: n }); };
  const [simHost, setSimHost] = useState('');
  const [sim, setSim] = useState<{ packages: number; assessment: Assessment } | null>(null);
  const simAct = useAction();

  const runSim = () => simAct.run(async () => setSim(await api('POST', '/policies/simulate', { host_id: simHost, definition: d })));

  return (
    <div className="stack stack--lg">
      <div className="form-grid">
        <Field label={t('pol.name')}>{(i) => <TextInput id={i} value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} required />}</Field>
        <Field label={t('pol.priority')} hint={t('pol.priority.hint')}>{(i) => <TextInput id={i} type="number" value={form.priority} onChange={(e) => setForm({ ...form, priority: Number(e.target.value) })} />}</Field>
        <div className="span-all"><Field label={t('common.description')}>{(i) => <TextInput id={i} value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />}</Field></div>
        <div className="span-all row">
          <Check checked={form.enabled} onChange={(v) => setForm({ ...form, enabled: v })}>{t('pol.enabled')}</Check>
          <Check checked={d.security_only} onChange={(v) => edit((n) => { n.security_only = v; })}>{t('pol.security_only')}</Check>
        </div>
        <fieldset className="fieldset span-all">
          <legend>{t('pol.groups')}</legend>
          <div className="chips">{groups.map((g) => <GroupTag key={g.id} group={g} active={form.group_ids.includes(g.id)} onClick={() => setForm({ ...form, group_ids: form.group_ids.includes(g.id) ? form.group_ids.filter((x) => x !== g.id) : [...form.group_ids, g.id] })} />)}</div>
          <p className="field__hint" style={{ marginTop: 8 }}>{t('pol.groups.hint')}</p>
        </fieldset>
        <div className="span-all"><Field label={t('pol.scope')} hint={t('pol.scope.hint')}>{(i) => <TextInput id={i} className="input--code" value={d.target_scope} onChange={(e) => edit((n) => { n.target_scope = e.target.value; })} spellCheck={false} />}</Field></div>
      </div>

      <section className="panel" style={{ padding: 16 }}>
        <h3 className="label" style={{ marginBottom: 12 }}>{t('pol.section.when')}</h3>
        <PatchWhen d={d} edit={edit} />
      </section>

      <Section title={t('pol.section.risk')}>
        <div className="stack">
          <Check checked={d.ai_mode_enabled} onChange={(v) => edit((n) => { n.ai_mode_enabled = v; })}>{t('pol.ai_mode')}</Check>
          <p className="field__hint">{t('pol.ai_mode.hint')}</p>
          <h3 className="label">{t('pol.rules')}</h3>
          <p className="field__hint">{t('pol.rules.hint')}</p>
          {d.rules.map((r, i) => (
            <div key={i} className="form-grid" style={{ gridTemplateColumns: 'minmax(0,1fr) minmax(0,3fr) minmax(0,1fr) auto', alignItems: 'end' }}>
              <Field label={t('pol.rule.id')}>{(id) => <TextInput id={id} value={r.rule_id} onChange={(e) => edit((n) => { n.rules[i]!.rule_id = e.target.value; })} />}</Field>
              <Field label={t('pol.rule.condition')}>{(id) => <TextInput id={id} className="input--code" value={r.condition} spellCheck={false} onChange={(e) => edit((n) => { n.rules[i]!.condition = e.target.value; })} />}</Field>
              <Field label={t('pol.rule.risk')}>{(id) => <RiskSelect id={id} value={r.contextual_risk} onChange={(v) => edit((n) => { n.rules[i]!.contextual_risk = v ?? 'Low'; })} />}</Field>
              <Button small aria-label={t('common.remove')} onClick={() => edit((n) => { n.rules.splice(i, 1); })}>{t('common.remove')}</Button>
            </div>
          ))}
          <div><Button small onClick={() => edit((n) => { n.rules.push({ rule_id: `rule_${n.rules.length + 1}`, condition: "cvss_score >= 8.0 AND asset_environment == 'production'", contextual_risk: 'Critical' }); })}>{t('pol.rule.add')}</Button></div>
        </div>
      </Section>

      <Section defaultOpen title={t('pol.section.decisions')}>
        <div className="stack">
          <p className="field__hint">{t('pol.decisions.hint')}</p>
          {d.decisions.map((x, i) => (
            <div key={i} className="fieldset">
              <div className="form-grid">
                <Field label={t('pol.decision.risk_eq')}>{(id) => <RiskSelect id={id} any value={x.conditions.contextual_risk} onChange={(v) => edit((n) => { n.decisions[i]!.conditions.contextual_risk = v; })} />}</Field>
                <Field label={t('pol.decision.risk_min')}>{(id) => <RiskSelect id={id} any value={x.conditions.contextual_risk_min} onChange={(v) => edit((n) => { n.decisions[i]!.conditions.contextual_risk_min = v; })} />}</Field>
                <Field label={t('pol.decision.stability_eq')}>{(id) => <RiskSelect id={id} any value={x.conditions.stability_risk} onChange={(v) => edit((n) => { n.decisions[i]!.conditions.stability_risk = v; })} />}</Field>
                <Field label={t('pol.decision.stability_max')}>{(id) => <RiskSelect id={id} any value={x.conditions.stability_risk_max} onChange={(v) => edit((n) => { n.decisions[i]!.conditions.stability_risk_max = v; })} />}</Field>
                <Field label={t('pol.decision.then')}>{(id) => <Select id={id} value={x.action} onChange={(e) => edit((n) => { n.decisions[i]!.action = e.target.value as PolicyAction; })}>{ACTIONS.map((a) => <option key={a} value={a}>{tr('action', a)}</option>)}</Select>}</Field>
                <div style={{ alignSelf: 'end' }}><Button small onClick={() => edit((n) => { n.decisions.splice(i, 1); })}>{t('common.remove')}</Button></div>
              </div>
            </div>
          ))}
          <div><Button small onClick={() => edit((n) => { n.decisions.push({ conditions: { contextual_risk_min: 'High' }, action: 'AUTO_DEPLOY' }); })}>{t('pol.decision.add')}</Button></div>
          <Field label={t('pol.default_action')}>{(id) => <Select id={id} value={d.default_action} onChange={(e) => edit((n) => { n.default_action = e.target.value as PolicyAction; })}>{ACTIONS.map((a) => <option key={a} value={a}>{tr('action', a)}</option>)}</Select>}</Field>
        </div>
      </Section>

      <Section title={t('pol.section.rings')}>
        <Field label={t('pol.ring_delay')}>{(id) => <TextInput id={id} type="number" min={0} value={d.rings.delay_minutes} onChange={(e) => edit((n) => { n.rings.delay_minutes = Number(e.target.value); })} />}</Field>
      </Section>

      <Section title={t('pol.section.reboot')}>
        <Field label={t('pol.reboot.mode')} hint={t('pol.reboot.hint')}>{(id) => (
          <Select id={id} value={d.reboot.mode || 'manual'} onChange={(e) => edit((n) => { n.reboot.mode = e.target.value === 'manual' ? '' : e.target.value; })}>
            <option value="manual">{t('pol.reboot.manual')}</option>
            <option value="auto">{t('pol.reboot.auto')}</option>
            <option value="user_prompt">{t('pol.reboot.user_prompt')}</option>
          </Select>)}</Field>
      </Section>
      <Section title={t('pol.section.rollback')}>
        <div className="stack">
          <div className="form-grid">
            <Field label={t('pol.soak')}>{(id) => <TextInput id={id} type="number" min={0} max={1440} value={d.rollback.soak_minutes} onChange={(e) => edit((n) => { n.rollback.soak_minutes = Number(e.target.value); })} />}</Field>
            <div className="row" style={{ alignSelf: 'end' }}>
              <Check checked={d.rollback.auto_rollback} onChange={(v) => edit((n) => { n.rollback.auto_rollback = v; })}>{t('pol.auto_rollback')}</Check>
              <Check checked={d.rollback.snapshot_required} onChange={(v) => edit((n) => { n.rollback.snapshot_required = v; })}>{t('pol.snapshot')}</Check>
            </div>
          </div>
          <h3 className="label">{t('pol.probes')}</h3>
          {d.rollback.probes.map((p, i) => (
            <div key={i} className="form-grid" style={{ gridTemplateColumns: 'minmax(0,1fr) minmax(0,3fr) auto', alignItems: 'end' }}>
              <Field label="Type">{(id) => (
                <Select id={id} value={p.type} onChange={(e) => edit((n) => { const ty = e.target.value as Probe['type']; n.rollback.probes[i] = ty === 'tcp' ? { type: 'tcp', host: '127.0.0.1', port: 443 } : ty === 'http' ? { type: 'http', url: 'http://127.0.0.1/', expect_status: 200 } : { type: 'systemd', unit: '' }; })}>
                  <option value="tcp">{t('pol.probe.tcp')}</option><option value="http">{t('pol.probe.http')}</option><option value="systemd">{t('pol.probe.systemd')}</option>
                </Select>)}</Field>
              {p.type === 'tcp' && <div className="form-grid"><Field label="Host">{(id) => <TextInput id={id} value={p.host ?? ''} onChange={(e) => edit((n) => { n.rollback.probes[i]!.host = e.target.value; })} />}</Field><Field label="Port">{(id) => <TextInput id={id} type="number" min={1} max={65535} value={p.port ?? 0} onChange={(e) => edit((n) => { n.rollback.probes[i]!.port = Number(e.target.value); })} />}</Field></div>}
              {p.type === 'http' && <div className="form-grid"><Field label="URL">{(id) => <TextInput id={id} value={p.url ?? ''} onChange={(e) => edit((n) => { n.rollback.probes[i]!.url = e.target.value; })} />}</Field><Field label="HTTP">{(id) => <TextInput id={id} type="number" value={p.expect_status ?? 200} onChange={(e) => edit((n) => { n.rollback.probes[i]!.expect_status = Number(e.target.value); })} />}</Field></div>}
              {p.type === 'systemd' && <Field label={t('pol.probe.systemd')}>{(id) => <TextInput id={id} value={p.unit ?? ''} placeholder="nginx.service" onChange={(e) => edit((n) => { n.rollback.probes[i]!.unit = e.target.value; })} />}</Field>}
              <Button small onClick={() => edit((n) => { n.rollback.probes.splice(i, 1); })}>{t('common.remove')}</Button>
            </div>
          ))}
          <div><Button small onClick={() => edit((n) => { n.rollback.probes.push({ type: 'tcp', host: '127.0.0.1', port: 443 }); })}>{t('pol.probe.add')}</Button></div>
        </div>
      </Section>

      <Section title={t('pol.simulate')}>
        <div className="stack">
          <p className="field__hint">{t('pol.simulate.hint')}</p>
          <div className="row">
            <Field label={t('pol.simulate.host')}>{(id) => <Select id={id} value={simHost} onChange={(e) => { setSimHost(e.target.value); setSim(null); }}><option value="" />{hosts.map((h) => <option key={h.id} value={h.id}>{h.hostname}</option>)}</Select>}</Field>
            <div style={{ alignSelf: 'end' }}><Button disabled={!simHost || simAct.pending} onClick={() => void runSim()}>{t('pol.simulate.run')}</Button></div>
          </div>
          {simAct.error && <Notice kind="error">{simAct.error}</Notice>}
          {sim && (
            <div className="stack" aria-live="polite">
              <div className="row">
                <Tag>{sim.packages} {t('pol.simulate.packages')}</Tag>
                <Tag>{t('tasks.contextual')} · {tr('risk', sim.assessment.contextual_risk)}</Tag>
                <Tag>{t('tasks.stability')} · {tr('risk', sim.assessment.stability_risk)}</Tag>
                <Tag>{tr('action', sim.assessment.action)}</Tag>
              </div>
              <ul style={{ margin: 0, paddingLeft: 20, fontSize: 13 }}>{sim.assessment.rationale.map((r, i) => <li key={i}>{r}</li>)}</ul>
            </div>
          )}
        </div>
      </Section>
    </div>
  );
}

/** Table cell: how and when the policy applies updates, and the next patch day. */
function When({ p }: { p: Policy }) {
  const { t, tr, date } = useI18n();
  const text = useWindowText();
  const w = p.definition.maintenance_window;
  const next = p.next_runs?.[0];
  return (
    <>
      {tr('action', p.definition.default_action)}
      {w ? <span className="sub">{text(w)} · {w.start}–{w.end}{w.timezone ? ` ${w.timezone}` : ` UTC${w.utc_offset_minutes ? (w.utc_offset_minutes > 0 ? '+' : '') + w.utc_offset_minutes / 60 : ''}`}</span> : <span className="sub">{t('pol.when.anytime')}</span>}
      {w && next && <span className="sub">{p.in_window ? t('pol.in_window') : t('pol.next_run', { d: date(next.start, { timeZone: safeZone(w.timezone), weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) })}</span>}
      <span className="sub">{t('pol.col.hosts', { n: p.hosts ?? 0 })}</span>
    </>
  );
}

function Policies() {
  const { t, tr } = useI18n();
  const { can } = useAuth();
  const list = useApi<Policy[]>('/policies');
  const groups = useApi<HostGroup[]>('/groups');
  const hosts = useApi<HostSummary[]>(can('hosts:read') ? '/hosts' : null);
  const [form, setForm] = useState<Form | null>(null);
  const [deleting, setDeleting] = useState<Policy | null>(null);
  const act = useAction();

  const open = (p?: Policy) => { act.clearError(); setForm(p ? { id: p.id, name: p.name, description: p.description, enabled: p.enabled, priority: p.priority, group_ids: p.group_ids, def: p.definition } : { name: '', description: '', enabled: true, priority: 100, group_ids: [], def: blankDef() }); };
  const save = () => act.run(async () => {
    const f = form!;
    const body = { name: f.name, description: f.description, enabled: f.enabled, priority: f.priority, group_ids: f.group_ids, definition: f.def };
    if (f.id) await api('PUT', `/policies/${f.id}`, body);
    else await api('POST', '/policies', body);
    setForm(null);
    list.reload();
  });
  const groupObj = (id: string) => groups.data?.find((g) => g.id === id) ?? { name: id.slice(0, 8) };

  return (
    <>
      <PageHeader title={t('pol.title')} subtitle={t('pol.subtitle')}
        actions={can('policies:create') && <Button variant="primary" onClick={() => open()}>{t('pol.new')}</Button>} />
      {list.error && <Notice kind="error">{list.error.message}</Notice>}
      {!list.data ? <Spinner /> : (
        <Table head={[t('pol.priority'), t('pol.name'), t('pol.groups'), t('pol.col.when'), t('common.status'), t('common.actions')]} empty={list.data.length === 0 && <Empty>{t('pol.none')}</Empty>}>
          {list.data.map((p) => (
            <tr key={p.id}>
              <td className="num">{p.priority}</td>
              <td className="wrap">{p.name}<span className="sub">{p.description}</span></td>
              <td className="wrap">{p.group_ids.length ? <div className="pkgs">{p.group_ids.map((g) => <GroupTag key={g} group={groupObj(g)} />)}</div> : <span className="muted">—</span>}</td>
              <td className="wrap"><When p={p} /></td>
              <td><Tag tone={p.enabled ? 'ok' : 'neutral'}>{p.enabled ? t('pol.enabled') : t('users.disabled')}</Tag></td>
              <td><div className="row row--tight">
                {can('policies:write') && <Button small onClick={() => open(p)}>{t('common.edit')}</Button>}
                {can('policies:delete') && <Button small onClick={() => setDeleting(p)}>{t('common.delete')}</Button>}
              </div></td>
            </tr>
          ))}
        </Table>
      )}
      <Drawer open={!!form} title={form?.id ? form.name : t('pol.new')} onClose={() => setForm(null)}>
        {form && (
          <form onSubmit={(e) => { e.preventDefault(); void save(); }} className="stack stack--lg">
            <Editor form={form} setForm={setForm} groups={groups.data ?? []} hosts={hosts.data ?? []} />
            {act.error && <Notice kind="error">{act.error}</Notice>}
            <div className="sticky-actions"><Button variant="primary" type="submit" disabled={act.pending || !can(form.id ? 'policies:write' : 'policies:create')}>{t('common.save')}</Button><Button variant="ghost" onClick={() => setForm(null)}>{t('common.cancel')}</Button></div>
          </form>
        )}
      </Drawer>
      <ConfirmDialog danger message={deleting ? t('pol.confirm_delete') : null} onNo={() => setDeleting(null)} onYes={() => { const p = deleting!; setDeleting(null); void act.run(async () => { await api('DELETE', `/policies/${p.id}`); list.reload(); }); }} />
    </>
  );
}

export default function Page() {
  return <><Policies /></>;
}
