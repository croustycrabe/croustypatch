'use client';
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { en, fr, type MessageKey } from './messages';
import type { Lang } from './types';

const tables: Record<Lang, Record<MessageKey, string>> = { fr, en };
const STORAGE = 'cp_lang';

interface Ctx {
  lang: Lang;
  setLang: (l: Lang) => void;
  t: (key: MessageKey, vars?: Record<string, string | number>) => string;
  /** Looks up a key built at runtime (status codes etc.); falls back to the raw value. */
  tr: (prefix: string, value: string) => string;
  date: (iso: string | null | undefined, opts?: Intl.DateTimeFormatOptions) => string;
  ago: (iso: string | null | undefined) => string;
}

const I18n = createContext<Ctx | null>(null);

function initialLang(): Lang {
  try {
    const s = localStorage.getItem(STORAGE);
    if (s === 'fr' || s === 'en') return s;
  } catch {
    /* storage unavailable */
  }
  return typeof navigator !== 'undefined' && navigator.language.toLowerCase().startsWith('en') ? 'en' : 'fr';
}

export function I18nProvider({ children }: { children: ReactNode }) {
  // French is the default; the stored/browser language is applied after hydration to keep SSR output stable.
  const [lang, setLangState] = useState<Lang>('fr');
  useEffect(() => setLangState(initialLang()), []);
  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  const setLang = useCallback((l: Lang) => {
    setLangState(l);
    try {
      localStorage.setItem(STORAGE, l);
    } catch {
      /* ignore */
    }
  }, []);

  const value = useMemo<Ctx>(() => {
    const table = tables[lang];
    const t: Ctx['t'] = (key, vars) => {
      let s: string = table[key];
      if (vars) for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
      return s;
    };
    const tr: Ctx['tr'] = (prefix, value) => {
      const k = `${prefix}.${value}` as MessageKey;
      return k in table ? table[k] : value;
    };
    const locale = lang === 'fr' ? 'fr-FR' : 'en-GB';
    const date: Ctx['date'] = (iso, opts) => (iso ? new Intl.DateTimeFormat(locale, opts ?? { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(iso)) : '—');
    const ago: Ctx['ago'] = (iso) => {
      if (!iso) return t('common.never');
      const secs = Math.round((new Date(iso).getTime() - Date.now()) / 1000);
      if (Math.abs(secs) < 45) return t('time.now');
      const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
      const units: [Intl.RelativeTimeFormatUnit, number][] = [['day', 86400], ['hour', 3600], ['minute', 60]];
      for (const [u, n] of units) if (Math.abs(secs) >= n) return rtf.format(Math.round(secs / n), u);
      return rtf.format(secs, 'second');
    };
    return { lang, setLang, t, tr, date, ago };
  }, [lang, setLang]);

  return <I18n.Provider value={value}>{children}</I18n.Provider>;
}

export function useI18n(): Ctx {
  const c = useContext(I18n);
  if (!c) throw new Error('useI18n outside I18nProvider');
  return c;
}
