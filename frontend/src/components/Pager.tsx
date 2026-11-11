'use client';
import { CaretLeft, CaretRight } from '@phosphor-icons/react';
import { useState } from 'react';
import { useI18n } from '@/lib/i18n';
import { Button, Select } from './ui';

export const PAGE_SIZES = [25, 50];
const KEY = 'cp.pagesize';

/** The number of rows per page, kept for this browser (25 or 50). */
export function usePageSize(): [number, (n: number) => void] {
  const [size, setSize] = useState<number>(() => {
    try {
      const v = Number(window.localStorage.getItem(KEY));
      return PAGE_SIZES.includes(v) ? v : PAGE_SIZES[0]!;
    } catch { return PAGE_SIZES[0]!; }
  });
  const set = (n: number) => {
    setSize(n);
    try { window.localStorage.setItem(KEY, String(n)); } catch { /* private window: the choice lasts until the page is left */ }
  };
  return [size, set];
}

/** Rows `from`–`to` of `total`, the page size (25 or 50) and previous / next. `page` is 0-based. Nothing to show for an empty list. */
export function Pager({ page, perPage, total, onPage, onPerPage, note }: { page: number; perPage: number; total: number; onPage: (p: number) => void; onPerPage: (n: number) => void; note?: string }) {
  const { t } = useI18n();
  if (total === 0) return null;
  const pages = Math.max(1, Math.ceil(total / perPage));
  const from = page * perPage + 1;
  const to = Math.min((page + 1) * perPage, total);
  return (
    <nav className="row pager" aria-label={t('pager.label')} style={{ justifyContent: 'space-between', marginTop: 12 }}>
      <span className="muted" style={{ fontSize: 13 }}>{t('pager.range', { from, to, total })}{note ? ` · ${note}` : ''}</span>
      <span className="row row--tight">
        <label className="row row--tight muted" style={{ fontSize: 13 }}>
          {t('pager.per_page')}
          <Select aria-label={t('pager.per_page')} value={perPage} onChange={(e) => { onPerPage(Number(e.target.value)); onPage(0); }} style={{ width: 'auto' }}>
            {PAGE_SIZES.map((n) => <option key={n} value={n}>{n}</option>)}
          </Select>
        </label>
        {pages > 1 && (
          <>
            <Button small disabled={page <= 0} onClick={() => onPage(page - 1)} icon={<CaretLeft size={14} weight="bold" />}>{t('common.previous')}</Button>
            <span className="muted" style={{ fontSize: 13 }} aria-live="polite">{t('pager.page', { page: page + 1, pages })}</span>
            <Button small disabled={page + 1 >= pages} onClick={() => onPage(page + 1)}>{t('common.next')}<CaretRight size={14} weight="bold" style={{ marginLeft: 4 }} /></Button>
          </>
        )}
      </span>
    </nav>
  );
}
