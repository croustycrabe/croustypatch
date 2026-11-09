'use client';
import { CaretDown, CaretUp, CheckCircle, Info, Tray, Warning, X, XCircle } from '@phosphor-icons/react';
import Link from 'next/link';
import { useEffect, useId, useRef, useState, type ButtonHTMLAttributes, type CSSProperties, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from 'react';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import type { HostStatus, TaskStatus } from '@/lib/types';

// ----------------------------------------------------------------- buttons
type Variant = 'primary' | 'outline' | 'ink' | 'ghost' | 'danger';

export function Button({ variant = 'outline', small, icon, className = '', children, ...p }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; small?: boolean; icon?: ReactNode }) {
  const iconOnly = !!icon && children === undefined;
  return (
    <button type="button" {...p} className={`btn btn--${variant} ${small ? 'btn--sm' : ''} ${iconOnly ? 'btn--icon' : ''} ${className}`}>
      {icon}
      {children}
    </button>
  );
}

export function LinkButton({ href, variant = 'outline', small, icon, children }: { href: string; variant?: Variant; small?: boolean; icon?: ReactNode; children: ReactNode }) {
  return (
    <Link href={href} className={`btn btn--${variant} ${small ? 'btn--sm' : ''}`}>
      {icon}
      {children}
    </Link>
  );
}

export function Chip({ active, count, children, ...p }: ButtonHTMLAttributes<HTMLButtonElement> & { active: boolean; count?: number }) {
  return (
    <button type="button" {...p} aria-pressed={active} className={`chip ${p.className ?? ''}`}>
      {children}
      {count !== undefined && <span className="n">{count}</span>}
    </button>
  );
}

export type Tone = 'ok' | 'warn' | 'info' | 'danger' | 'neutral';

/** The state icon that always goes with a state colour (design rule: never colour alone). */
export function StateIcon({ tone, size = 12 }: { tone: Tone; size?: number }) {
  const p = { size, weight: 'bold' as const, 'aria-hidden': true, className: 'icon' };
  if (tone === 'ok') return <CheckCircle {...p} />;
  if (tone === 'warn') return <Warning {...p} />;
  if (tone === 'danger') return <XCircle {...p} />;
  if (tone === 'info') return <Info {...p} />;
  return null;
}

export function Tag({ children, tone }: { children: ReactNode; tone?: Tone | 'brick' }) {
  const t = tone === 'brick' ? 'danger' : tone;
  return <span className={`tag ${t ? `tone-${t}` : ''}`}>{t && <StateIcon tone={t} />}{children}</span>;
}

// ------------------------------------------------------------ group colours
export const GROUP_COLORS = ['brick', 'sand', 'moss', 'teal', 'sky', 'plum', 'rose', 'slate', 'ink'] as const;
export type GroupColor = (typeof GROUP_COLORS)[number];
const HEX = /^#[0-9a-fA-F]{6}$/;
export const isHexColor = (c: string | undefined): c is string => !!c && HEX.test(c);

/** The colour a group is drawn with: a palette name, a custom `#rrggbb`, or a stable pick derived from its name. */
export function groupColor(g: { name: string; color?: string }): string {
  if (g.color && ((GROUP_COLORS as readonly string[]).includes(g.color) || isHexColor(g.color))) return g.color;
  let h = 0;
  for (const ch of g.name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return GROUP_COLORS[h % GROUP_COLORS.length] ?? 'slate';
}

/** Ink or paper, whichever reads better on top of this colour. */
function readableOn(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255) as [number, number, number];
  return 0.299 * r + 0.587 * g + 0.114 * b > 0.6 ? 'var(--ink)' : 'var(--paper)';
}

const colorVar = (c: string) => (isHexColor(c) ? { ['--c' as string]: c, ['--on' as string]: readableOn(c) } : { ['--c' as string]: `var(--g-${c})` }) as CSSProperties;

export function GroupTag({ group, active, onClick, disabled }: { group: { name: string; color?: string }; active?: boolean; onClick?: () => void; disabled?: boolean }) {
  const style = colorVar(groupColor(group));
  if (!onClick) return <span className="gtag" style={style}><i />{group.name}</span>;
  return (
    <button type="button" className="gtag" style={style} aria-pressed={!!active} disabled={disabled} onClick={onClick}>
      <i />{group.name}
    </button>
  );
}

/** Swatch picker: automatic, nine palette colours, or any colour from the system picker. `value` '' means automatic. */
export function ColorPicker({ value, onChange, name, disabled, live }: { value: string; onChange: (c: string) => void; name: string; disabled?: boolean; live?: boolean }) {
  const { t } = useI18n();
  const custom = isHexColor(value);
  const [draft, setDraft] = useState(custom ? value : '#4d6169');
  const input = useRef<HTMLInputElement>(null);
  const commit = useRef(onChange);
  commit.current = onChange;
  useEffect(() => { if (isHexColor(value)) setDraft(value); }, [value]);
  // The native picker fires "input" while dragging and "change" when it closes: outside a live preview only the
  // final colour is committed, so one choice makes one save.
  useEffect(() => {
    const el = input.current;
    if (!el || live) return;
    const done = () => commit.current(el.value.toLowerCase());
    el.addEventListener('change', done);
    return () => el.removeEventListener('change', done);
  }, [live]);
  const shown = draft; // the input follows the draft, so the final "change" event carries the colour just picked
  return (
    <div className="swatches" role="group" aria-label={t('groups.color')}>
      <button type="button" className="swatch swatch--auto" aria-pressed={value === ''} title={t('groups.color.auto')} aria-label={t('groups.color.auto')} disabled={disabled} onClick={() => onChange('')}>A</button>
      {GROUP_COLORS.map((c) => (
        <button key={c} type="button" className="swatch" style={colorVar(c)} aria-pressed={value === c} title={t(`groups.color.${c}` as MessageKey)} aria-label={t(`groups.color.${c}` as MessageKey)} disabled={disabled} onClick={() => onChange(c)} />
      ))}
      <label className={`swatch swatch--custom ${custom ? 'is-set' : ''}`} style={custom ? colorVar(value) : undefined} title={t('groups.color.custom')}>
        <input ref={input} type="color" value={shown} disabled={disabled} aria-label={t('groups.color.custom')}
          onChange={(e) => { const v = e.target.value.toLowerCase(); setDraft(v); if (live) onChange(v); }} />
      </label>
      <span className="sr-only">{name}</span>
    </div>
  );
}

export function Badge({ children }: { children: ReactNode }) {
  return <span className="badge">{children}</span>;
}

// ----------------------------------------------------------------- status (shape + label, never colour alone)
export type Dot = 'solid' | 'hollow' | 'half' | 'dashed' | 'alert' | 'isolated';

export const HOST_DOT: Record<HostStatus, Dot> = { online: 'solid', offline: 'hollow', eco: 'half', maintenance: 'dashed', alert: 'alert', quarantined: 'isolated', pending: 'dashed' };
/** online = green, offline = grey, eco/maintenance = blue/amber, alert/quarantined = red. */
export const HOST_TONE: Record<HostStatus, Tone> = { online: 'ok', offline: 'neutral', eco: 'info', maintenance: 'warn', alert: 'danger', quarantined: 'danger', pending: 'neutral' };
const TASK_DOT: Record<TaskStatus, Dot> = {
  pending_approval: 'dashed', queued: 'hollow', sent: 'half', running: 'half', soaking: 'half', succeeded: 'solid', failed: 'alert',
  rolling_back: 'half', rolled_back: 'alert', quarantined: 'isolated', blocked: 'hollow', rejected: 'hollow', cancelled: 'hollow', expired: 'hollow',
};
export const TASK_TONE: Record<TaskStatus, Tone> = {
  pending_approval: 'warn', queued: 'neutral', sent: 'info', running: 'info', soaking: 'info', succeeded: 'ok', failed: 'danger',
  rolling_back: 'warn', rolled_back: 'warn', quarantined: 'danger', blocked: 'danger', rejected: 'neutral', cancelled: 'neutral', expired: 'neutral',
};

export function StatusDot({ kind, tone }: { kind: Dot; tone?: Tone }) {
  return <span className={`dot dot--${kind} ${tone ? `tone-${tone}` : ''}`} aria-hidden="true" />;
}

export function HostStatusLabel({ status }: { status: HostStatus }) {
  const { tr } = useI18n();
  const tone = HOST_TONE[status] ?? 'neutral';
  return (
    <span className={`status tone-${tone}`}>
      <StatusDot kind={HOST_DOT[status] ?? 'hollow'} />
      {tr('status', status)}
    </span>
  );
}

export function TaskStatusLabel({ status }: { status: TaskStatus }) {
  const { tr } = useI18n();
  const tone = TASK_TONE[status] ?? 'neutral';
  return (
    <span className={`status tone-${tone}`}>
      <StatusDot kind={TASK_DOT[status] ?? 'hollow'} />
      {tr('task', status)}
    </span>
  );
}

// ------------------------------------------------------------------ layout
export function PageHeader({ title, subtitle, actions, back, badge }: { title: ReactNode; subtitle?: string; actions?: ReactNode; back?: { href: string; label: string }; badge?: ReactNode }) {
  return (
    <header className="page-head">
      <div>
        {back && (
          <Link href={back.href} className="page-head__back">
            ← {back.label}
          </Link>
        )}
        <h1 className="page-head__title">
          <span><span className="br" aria-hidden="true">[</span> {title} <span className="br" aria-hidden="true">]</span></span>
          {badge}
        </h1>
        {subtitle && <p className="page-head__sub">{subtitle}</p>}
      </div>
      {actions && <div className="page-head__actions">{actions}</div>}
    </header>
  );
}

export function Card({ title, actions, children, className = '', flush }: { title?: ReactNode; actions?: ReactNode; children: ReactNode; className?: string; flush?: boolean }) {
  return (
    <section className={`panel ${flush ? 'panel--flush' : ''} ${className}`}>
      {(title || actions) && (
        <div className="panel__head">
          {title && <h2 className="panel__title">{title}</h2>}
          {actions && <div className="row row--tight">{actions}</div>}
        </div>
      )}
      <div className="panel__body">{children}</div>
    </section>
  );
}

/** Collapsible group of fields (native <details>: keyboard and screen-reader friendly). */
export function Section({ title, children, defaultOpen = false }: { title: ReactNode; children: ReactNode; defaultOpen?: boolean }) {
  return (
    <details className="section" open={defaultOpen}>
      <summary>
        {title}
        <CaretDown size={14} weight="bold" className="caret icon" />
      </summary>
      <div className="section__body">{children}</div>
    </details>
  );
}

export function Notice({ kind = 'info', children }: { kind?: 'info' | 'error' | 'ok'; children: ReactNode }) {
  return (
    <div className={`alert alert--${kind}`} role={kind === 'error' ? 'alert' : 'status'}>
      <StateIcon tone={kind === 'error' ? 'danger' : kind === 'ok' ? 'ok' : 'info'} size={20} />
      <div className="alert__body">{children}</div>
    </div>
  );
}

export function Empty({ children, title, icon, action }: { children?: ReactNode; title?: string; icon?: ReactNode; action?: ReactNode }) {
  return (
    <div className="empty">
      <div className="empty__icon">{icon ?? <Tray size={22} weight="bold" />}</div>
      {title && <p className="empty__title">{title}</p>}
      {children && <p className="empty__text">{children}</p>}
      {action && <div className="empty__action">{action}</div>}
    </div>
  );
}

export function Skeleton({ width, height }: { width?: number | string; height?: number }) {
  return <span className="skeleton" style={{ width, height }} aria-hidden="true" />;
}

/** Loading placeholder: shimmering rows instead of a spinner, with a text alternative. */
export function Spinner() {
  const { t } = useI18n();
  return (
    <div className="skeletons" role="status">
      <span className="sr-only">{t('common.loading')}</span>
      <Skeleton width="38%" height={16} />
      <Skeleton width="100%" />
      <Skeleton width="92%" />
      <Skeleton width="76%" />
    </div>
  );
}

export function StatCard({ value, label, alert, tone, href }: { value: ReactNode; label: string; alert?: boolean; tone?: 'ok' | 'warn' | 'danger'; href?: string }) {
  const t = alert ? 'danger' : tone;
  const inner = (
    <>
      <span className={`stat__n ${t === 'danger' ? 'stat__n--alert' : t ? `stat__n--${t}` : ''}`}>{value}</span>
      <span className="stat__l">{label}</span>
    </>
  );
  return href ? (
    <Link href={href} className="stat">
      {inner}
    </Link>
  ) : (
    <div className="stat">{inner}</div>
  );
}

// ------------------------------------------------------------------ forms
export function Field({ label, hint, children }: { label: string; hint?: string; children: (id: string) => ReactNode }) {
  const id = useId();
  return (
    <div className="field">
      <label htmlFor={id} className="label">
        {label}
      </label>
      {children(id)}
      {hint && <p className="field__hint">{hint}</p>}
    </div>
  );
}

export const TextInput = (p: InputHTMLAttributes<HTMLInputElement>) => <input {...p} className={`input ${p.className ?? ''}`} />;
export const TextArea = (p: TextareaHTMLAttributes<HTMLTextAreaElement>) => <textarea {...p} className={`input input--area ${p.className ?? ''}`} />;
export const Select = (p: SelectHTMLAttributes<HTMLSelectElement>) => <select {...p} className={`input ${p.className ?? ''}`} />;

export function SearchInput({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <div className="search">
      <svg className="icon" width="14" height="14" viewBox="0 0 256 256" aria-hidden="true"><path fill="currentColor" d="M232.5 215.5 185.4 168.4a92.1 92.1 0 1 0-17 17l47.1 47.1a12 12 0 0 0 17-17ZM44 112a68 68 0 1 1 68 68 68.1 68.1 0 0 1-68-68Z" /></svg>
      <input type="search" className="input" aria-label={placeholder} placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />
    </div>
  );
}

export function Check({ checked, onChange, children, disabled }: { checked: boolean; onChange: (v: boolean) => void; children: ReactNode; disabled?: boolean }) {
  return (
    <label className={`check ${disabled ? 'check--disabled' : ''}`}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>{children}</span>
    </label>
  );
}

export function Switch({ checked, onChange, children, disabled }: { checked: boolean; onChange: (v: boolean) => void; children: ReactNode; disabled?: boolean }) {
  return (
    <button type="button" role="switch" aria-checked={checked} disabled={disabled} className="switch" onClick={() => onChange(!checked)}>
      <span className="switch__track" />
      <span>{children}</span>
    </button>
  );
}

// ------------------------------------------------------------------ tabs and menu
export function Tabs<T extends string>({ tabs, value, onChange }: { tabs: { id: T; label: string; count?: number }[]; value: T; onChange: (id: T) => void }) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((t) => (
        <button key={t.id} type="button" role="tab" aria-selected={value === t.id} className="tab" onClick={() => onChange(t.id)}>
          {t.label}
          {t.count !== undefined && <span className="n">{t.count}</span>}
        </button>
      ))}
    </div>
  );
}

/** Dropdown menu closed by Escape or a click outside. */
export function Menu({ label, trigger, children, up }: { label: string; trigger: ReactNode; children: (close: () => void) => ReactNode; up?: boolean }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', esc);
    return () => {
      document.removeEventListener('mousedown', away);
      document.removeEventListener('keydown', esc);
    };
  }, [open]);
  return (
    <div className="menu" ref={ref} style={up ? { display: 'block' } : undefined}>
      <button type="button" className={up ? 'usercard' : 'btn btn--outline btn--icon'} aria-haspopup="menu" aria-expanded={open} aria-label={label} onClick={() => setOpen((o) => !o)}>
        {trigger}
      </button>
      {open && (
        <div className={`menu__panel ${up ? 'menu__panel--up' : ''}`} role="menu">
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

export function MenuItem({ icon, danger, children, ...p }: ButtonHTMLAttributes<HTMLButtonElement> & { icon?: ReactNode; danger?: boolean }) {
  return (
    <button type="button" role="menuitem" {...p} className={`menu__item ${danger ? 'menu__item--danger' : ''}`}>
      {icon}
      {children}
    </button>
  );
}

// ------------------------------------------------------------------ overlays
function useModal(open: boolean) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);
  return ref;
}

export function Dialog({ open, title, onClose, children, wide }: { open: boolean; title: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  const ref = useModal(open);
  const { t } = useI18n();
  return (
    <dialog ref={ref} className={`dialog ${wide ? 'dialog--wide' : ''}`} onClose={onClose} onClick={(e) => e.target === ref.current && onClose()}>
      {open && (
        <div className="dialog__inner">
          <div className="panel__head">
            <h2 className="panel__title">{title}</h2>
            <Button variant="ghost" small icon={<X size={16} weight="bold" />} onClick={onClose} aria-label={t('common.close')} />
          </div>
          <div className="panel__body">{children}</div>
        </div>
      )}
    </dialog>
  );
}

/** Slide-over panel from the right edge: for forms and detail views that should not hide the page behind. */
export function Drawer({ open, title, subtitle, onClose, children }: { open: boolean; title: ReactNode; subtitle?: string; onClose: () => void; children: ReactNode }) {
  const ref = useModal(open);
  const { t } = useI18n();
  return (
    <dialog ref={ref} className="drawer" onClose={onClose} onClick={(e) => e.target === ref.current && onClose()}>
      {open && (
        <div className="drawer__inner">
          <div className="panel__head">
            <div>
              <h2 className="panel__title">{title}</h2>
              {subtitle && <p className="field__hint">{subtitle}</p>}
            </div>
            <Button variant="ghost" small icon={<X size={16} weight="bold" />} onClick={onClose} aria-label={t('common.close')} />
          </div>
          <div className="panel__body">{children}</div>
        </div>
      )}
    </dialog>
  );
}

export function ConfirmDialog({ message, onYes, onNo, danger }: { message: string | null; onYes: () => void; onNo: () => void; danger?: boolean }) {
  const { t } = useI18n();
  return (
    <Dialog open={message !== null} title={t('common.confirm')} onClose={onNo}>
      <div className="stack">
        <p>{message}</p>
        <div className="row">
          <Button variant={danger ? 'primary' : 'ink'} onClick={onYes}>
            {t('common.confirm')}
          </Button>
          <Button variant="ghost" onClick={onNo}>
            {t('common.cancel')}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

// ------------------------------------------------------------------ table
export function Table({ head, children, empty }: { head: ReactNode[]; children: ReactNode; empty?: ReactNode }) {
  return (
    <div className="tbl-wrap">
      <table className="tbl">
        <thead>
          <tr>
            {head.map((h, i) => (
              <th key={i} scope="col">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
      {empty}
    </div>
  );
}

/** Column header that sorts when clicked. */
export function SortHead({ label, active, dir, onClick }: { label: string; active: boolean; dir: 'asc' | 'desc'; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} aria-label={label}>
      {label}
      {active && (dir === 'asc' ? <CaretUp size={11} weight="bold" /> : <CaretDown size={11} weight="bold" />)}
    </button>
  );
}

