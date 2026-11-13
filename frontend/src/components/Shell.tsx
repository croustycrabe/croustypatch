'use client';
import { Bell, Cube, ClipboardText, Scroll, SquaresFour, Gear, HardDrives, House, Key, List, ListChecks, MagnifyingGlass, Package, Scales, ShieldCheck, ShieldWarning, SignOut, UserGear, UsersThree, Lock, Sun, Moon, type Icon } from '@phosphor-icons/react';
import Link from 'next/link';
import { usePathname, useRouter } from 'next/navigation';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import type { MessageKey } from '@/lib/messages';
import type { Dashboard, HostSummary, Lang } from '@/lib/types';
import { AddAgentButton, AddAgentProvider, useAddAgent } from './AddAgent';
import { Button, Menu, MenuItem, Spinner } from './ui';

interface NavItem { href: string; key: MessageKey; perm: string; icon: Icon }
export const NAV_GROUPS: { label: MessageKey; items: NavItem[] }[] = [
  { label: 'nav.group.overview', items: [{ href: '/', key: 'nav.dashboard', perm: 'dashboard:read', icon: House }] },
  {
    label: 'nav.group.fleet',
    items: [
      { href: '/hosts', key: 'nav.hosts', perm: 'hosts:read', icon: HardDrives },
      { href: '/packages', key: 'nav.packages', perm: 'hosts:read', icon: Package },
      { href: '/docker', key: 'nav.docker', perm: 'hosts:read', icon: Cube },
      { href: '/groups', key: 'nav.groups', perm: 'groups:read', icon: UsersThree },
      { href: '/portal', key: 'nav.portal', perm: 'hosts:read', icon: SquaresFour },
    ],
  },
  {
    label: 'nav.group.patching',
    items: [
      { href: '/tasks', key: 'nav.tasks', perm: 'tasks:read', icon: ListChecks },
      { href: '/policies', key: 'nav.policies', perm: 'policies:read', icon: ShieldCheck },
      { href: '/waivers', key: 'nav.waivers', perm: 'waivers:read', icon: Scales },
      { href: '/vulnerabilities', key: 'nav.vulns', perm: 'vulnerabilities:read', icon: ShieldWarning },
    ],
  },
  {
    label: 'nav.group.governance',
    items: [
    ],
  },
  {
    label: 'nav.group.admin',
    items: [
      { href: '/enrollment', key: 'nav.enrollment', perm: 'enrollment:read', icon: Key },
      { href: '/audit', key: 'nav.audit', perm: 'audit:read', icon: ClipboardText },
      { href: '/logs', key: 'nav.logs', perm: 'audit:read', icon: Scroll },
      { href: '/users', key: 'nav.users', perm: 'users:read', icon: UserGear },
      { href: '/notifications', key: 'nav.notifications', perm: 'settings:read', icon: Bell },
      { href: '/settings', key: 'nav.settings', perm: 'settings:read', icon: Gear },
    ],
  },
];
const ALL_ITEMS = NAV_GROUPS.flatMap((g) => g.items);

export function LanguageToggle() {
  const { lang, setLang, t } = useI18n();
  const { me } = useAuth();
  const choose = (l: Lang) => {
    setLang(l);
    if (me) api('PUT', '/auth/language', { language: l }).catch(() => undefined);
  };
  return (
    <div className="lang" role="group" aria-label={t('common.language')}>
      {(['fr', 'en'] as const).map((l) => (
        <button key={l} type="button" aria-pressed={lang === l} lang={l} onClick={() => choose(l)}>
          {l}
        </button>
      ))}
    </div>
  );
}

function Brand({ onDark }: { onDark?: boolean }) {
  const { t } = useI18n();
  return (
    <Link href="/" className="sidebar__brand" aria-label={t('app.name')} style={onDark ? { border: 0, padding: 0 } : undefined}>
      {/* The mark is used as supplied by the design system, never redrawn. */}
      {onDark
        ? <img src="/brand/crab-logo-mono-cream.svg" alt="" width={34} height={34} />
        : <>
            {/* One mark per theme: the brick crab on light, the cream one on dark. */}
            <img className="logo--light" src="/brand/crab-logo.svg" alt="" width={34} height={34} />
            <img className="logo--dark" src="/brand/crab-logo-dark.svg" alt="" width={34} height={34} />
          </>}
      <span className="brandmark">
        <span className="cl-bracket" aria-hidden="true">[</span> {t('app.name')} <span className="cl-bracket" aria-hidden="true">]</span>
      </span>
    </Link>
  );
}

/** The version of the running server (`GET /server-info`, public), "0.7.3"; empty until known or when it cannot be read. */
export function useServerVersion(): string {
  const info = useApi<{ version: string }>('/server-info');
  return info.data?.version ?? '';
}

/** Sign-in style pages: brand panel on the left, the form on the right. */
export function AuthLayout({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const version = useServerVersion();
  return (
    <div className="auth">
      <aside className="auth__brand">
        <img src="/brand/crab-logo-mono-cream.svg" alt="" width={84} height={84} />
        <div>
          <h2><span className="cl-bracket" aria-hidden="true">[</span> {t('app.name')} <span className="cl-bracket" aria-hidden="true">]</span></h2>
          <p>{t('app.pitch')}</p>
        </div>
        <small>Croustylab · Croustycrabe{version && <> · <span data-testid="app-version">v{version}</span></>}</small>
      </aside>
      <main className="auth__form">
        <div className="auth__lang"><LanguageToggle /></div>
        <div className="auth__card">{children}</div>
      </main>
    </div>
  );
}

/** Ctrl/Cmd+K: jump to a page or a host, or run a common action. */
function CommandPalette({ open, onClose, hosts }: { open: boolean; onClose: () => void; hosts: HostSummary[] | undefined }) {
  const { t, lang, setLang } = useI18n();
  const { can, logout } = useAuth();
  const router = useRouter();
  const addAgent = useAddAgent();
  const ref = useRef<HTMLDialogElement>(null);
  const [q, setQ] = useState('');
  const [sel, setSel] = useState(0);

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);
  // Start empty every time: clear on close, not after the next open (typing right away must not mix with the old query).
  useEffect(() => {
    if (!open) { setQ(''); setSel(0); }
  }, [open]);

  type Entry = { id: string; group: string; label: string; hint?: string; run: () => void };
  const entries = useMemo<Entry[]>(() => {
    const go = (href: string) => () => router.push(href);
    const e: Entry[] = ALL_ITEMS.filter((n) => can(n.perm)).map((n) => ({ id: n.href, group: t('palette.pages'), label: t(n.key), run: go(n.href) }));
    for (const h of hosts ?? []) e.push({ id: h.id, group: t('palette.hosts'), label: h.display_name || h.hostname, hint: h.distro, run: go(`/host?id=${h.id}`) });
    if (can('enrollment:create')) e.push({ id: 'add-agent', group: t('palette.actions'), label: t('nav.add_agent'), run: addAgent.open });
    e.push({ id: 'lang', group: t('palette.actions'), label: t('palette.switch_lang'), run: () => setLang(lang === 'fr' ? 'en' : 'fr') });
    e.push({ id: 'logout', group: t('palette.actions'), label: t('nav.logout'), run: () => void logout().then(() => router.replace('/login/')) });
    return e;
  }, [can, hosts, t, router, addAgent.open, lang, setLang, logout]);

  const shown = useMemo(() => {
    const n = q.trim().toLowerCase();
    return (n ? entries.filter((x) => x.label.toLowerCase().includes(n) || (x.hint ?? '').toLowerCase().includes(n)) : entries).slice(0, 30);
  }, [entries, q]);
  useEffect(() => setSel(0), [q]);

  const run = (x: Entry | undefined) => {
    if (!x) return;
    onClose();
    // let the palette close before a drawer opens on top of the page
    setTimeout(x.run, 0);
  };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); setSel((s) => Math.min(s + 1, shown.length - 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setSel((s) => Math.max(s - 1, 0)); }
    else if (e.key === 'Enter') { e.preventDefault(); run(shown[sel]); }
  };

  let lastGroup = '';
  return (
    <dialog ref={ref} className="palette" onClose={onClose} onClick={(e) => e.target === ref.current && onClose()} aria-label={t('nav.search')}>
      {open && (
        <>
          <input className="palette__input" autoFocus role="combobox" aria-expanded="true" aria-controls="palette-list" aria-activedescendant={shown[sel] ? `pal-${shown[sel]!.id}` : undefined} placeholder={t('palette.placeholder')} value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={onKey} />
          <ul className="palette__list" id="palette-list" role="listbox">
            {shown.length === 0 && <li className="palette__group">{t('palette.none')}</li>}
            {shown.map((x, i) => {
              const header = x.group !== lastGroup ? x.group : null;
              lastGroup = x.group;
              return (
                <li key={`${x.group}-${x.id}`} role="presentation">
                  {header && <div className="palette__group">{header}</div>}
                  <div id={`pal-${x.id}`} role="option" aria-selected={i === sel} className="palette__item" onMouseMove={() => setSel(i)} onClick={() => run(x)}>
                    {x.label}
                    {x.hint && <small>{x.hint}</small>}
                  </div>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </dialog>
  );
}

/** Light / dark switch; the choice is remembered, otherwise the system preference applies (see app/layout.tsx). */
export function ThemeToggle() {
  const { t } = useI18n();
  const [dark, setDark] = useState(false);
  useEffect(() => setDark(document.documentElement.dataset.theme === 'dark'), []);
  const flip = () => {
    const next = dark ? 'light' : 'dark';
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem('cp.theme', next); } catch { /* storage blocked: the choice lasts for this page */ }
    setDark(!dark);
  };
  return <Button variant="ghost" className="themebtn" icon={dark ? <Sun size={18} weight="bold" /> : <Moon size={18} weight="bold" />} aria-label={dark ? t('theme.light') : t('theme.dark')} title={dark ? t('theme.light') : t('theme.dark')} onClick={flip} />;
}

function Frame({ children }: { children: ReactNode }) {
  const { me, logout, can } = useAuth();
  const { t } = useI18n();
  const router = useRouter();
  const path = usePathname() ?? '/';
  const [navOpen, setNavOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    try { setCollapsed(localStorage.getItem('cp.nav.collapsed') === '1'); } catch { /* storage blocked: keep the default */ }
    const mq = window.matchMedia('(max-width: 940px)');
    const sync = () => setNarrow(mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  }, []);
  const toggleNav = () => {
    if (narrow) { setNavOpen((o) => !o); return; }
    setCollapsed((c) => {
      try { localStorage.setItem('cp.nav.collapsed', c ? '0' : '1'); } catch { /* storage blocked */ }
      return !c;
    });
  };
  useEffect(() => {
    if (!navOpen) return;
    const k = (e: KeyboardEvent) => e.key === 'Escape' && setNavOpen(false);
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [navOpen]);
  const [palette, setPalette] = useState(false);
  const version = useServerVersion();
  const dash = useApi<Dashboard>(can('dashboard:read') ? '/dashboard' : null, { poll: 20000 });
  // The palette searches hosts: have the list ready before it opens, and refresh it each time it does.
  const hostList = useApi<HostSummary[]>(can('hosts:read') ? '/hosts' : null);
  const reloadHosts = hostList.reload;

  useEffect(() => setNavOpen(false), [path]);
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); setPalette((o) => !o); void reloadHosts(); }
    };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [reloadHosts]);

  const isActive = (href: string) => (href === '/' ? path === '/' : path === href || path.startsWith(`${href}/`) || (href === '/hosts' && path.startsWith('/host')));
  const current = ALL_ITEMS.find((n) => isActive(n.href));
  const awaiting = dash.data?.tasks.awaiting_approval ?? 0;

  return (
    <div className={`shell ${collapsed ? 'shell--collapsed' : ''}`}>
      <a className="sr-only" href="#content">{t('common.skip')}</a>
      <aside id="app-sidebar" className={`sidebar ${navOpen ? 'sidebar--open' : ''}`}>
        <Brand />
        <div className="sidebar__cta"><AddAgentButton /></div>
        <nav className="sidebar__nav" aria-label={t('nav.label')}>
          {NAV_GROUPS.map((g) => {
            const items = g.items.filter((n) => can(n.perm));
            if (items.length === 0) return null;
            return (
              <div key={g.label} style={{ display: 'contents' }}>
                <div className="navgroup">{t(g.label)}</div>
                {items.map((n) => (
                  <Link key={n.href} href={n.href} className="navitem" aria-current={isActive(n.href) ? 'page' : undefined} title={collapsed && !narrow ? t(n.key) : undefined}>
                    <n.icon size={18} weight="bold" className="icon" />
                    <span className="navitem__label">{t(n.key)}</span>
                    {n.href === '/tasks' && awaiting > 0 && <span className="count" aria-label={`${awaiting}`}>{awaiting}</span>}
                  </Link>
                ))}
              </div>
            );
          })}
        </nav>
        <div className="sidebar__foot">
          {version && <div className="sidebar__version" title={t('app.version.title')}>{t('app.name')} <span data-testid="app-version">v{version}</span></div>}
          <Menu up label={me?.username ?? ''} trigger={<><span className="avatar">{me?.username.slice(0, 1)}</span><span style={{ minWidth: 0 }}><div className="usercard__name">{me?.username}</div><div className="usercard__role">{me?.role}</div></span></>}>
            {(close) => (
              <>
                <MenuItem icon={<Key size={16} weight="bold" />} onClick={() => { close(); router.push('/account/'); }}>{t('nav.account')}</MenuItem>
                <MenuItem icon={<Lock size={16} weight="bold" />} onClick={() => { close(); router.push('/password/'); }}>{t('user.change_password')}</MenuItem>
                <div className="menu__sep" />
                <MenuItem icon={<SignOut size={16} weight="bold" />} onClick={() => { close(); void logout().then(() => router.replace('/login/')); }}>{t('nav.logout')}</MenuItem>
              </>
            )}
          </Menu>
        </div>
      </aside>
      {navOpen && <div className="scrim" onClick={() => setNavOpen(false)} />}
      <div className="maincol">
        <header className="topbar">
          <Button variant="ghost" className="menu-btn" icon={<List size={20} weight="bold" />} aria-label={t('nav.menu')} aria-expanded={narrow ? navOpen : !collapsed} aria-controls="app-sidebar" onClick={toggleNav} />
          <div className="topbar__crumb">{current ? <b>{t(current.key)}</b> : <b>{t('app.name')}</b>}</div>
          <div className="topbar__tools">
            <button type="button" className="searchbtn" onClick={() => { setPalette(true); void reloadHosts(); }} aria-label={t('nav.search')}>
              <MagnifyingGlass size={15} weight="bold" className="icon" />
              <span>{t('nav.search')}</span>
              <kbd className="kbd">⌘K</kbd>
            </button>
            <ThemeToggle />
            <LanguageToggle />
          </div>
        </header>
        <main id="content" className="content">{children}</main>
      </div>
      <CommandPalette open={palette} onClose={() => setPalette(false)} hosts={hostList.data} />
    </div>
  );
}

export function Shell({ children }: { children: ReactNode }) {
  const { me, booting } = useAuth();
  const router = useRouter();
  useEffect(() => {
    if (booting) return;
    if (!me) router.replace('/login/');
    else if (me.must_change_password) router.replace('/password/');
  }, [booting, me, router]);

  if (booting || !me || me.must_change_password) {
    return <div className="content"><Spinner /></div>;
  }
  return (
    <AddAgentProvider>
      <Frame>{children}</Frame>
    </AddAgentProvider>
  );
}

