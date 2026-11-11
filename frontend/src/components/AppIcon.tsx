'use client';
import { AppWindow, ChartBar, ChatCircle, Code, Cube, Database, FileText, Globe, Image as ImageIcon, MusicNotes, ShieldCheck, Terminal, Trash, UploadSimple, Wrench, type Icon } from '@phosphor-icons/react';
import { useRef, useState } from 'react';
import { useApi } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';
import { Button, SearchInput, Select, Tabs } from './ui';

export interface BrandIcon { slug: string; title: string; hex: string; url: string }

/** Generic icons drawn by the interface (the server knows the same names). */
export const GENERIC_ICONS: Record<string, Icon> = { '': Cube, 'app-window': AppWindow, code: Code, terminal: Terminal, browser: Globe, 'file-text': FileText, 'chart-bar': ChartBar, 'shield-check': ShieldCheck, database: Database, image: ImageIcon, 'music-notes': MusicNotes, 'chat-circle': ChatCircle, wrench: Wrench, cube: Cube };

/** The icon tile of an app: its logo or uploaded image when it has one, else a generic glyph. */
export function AppIcon({ icon, url, size = 22 }: { icon: string; url?: string | null; size?: number }) {
  const src = url ?? (icon.startsWith('data:image/') ? icon : null);
  if (src) return <span className="appcard__icon appcard__icon--img" aria-hidden="true"><img src={src} alt="" width={size + 4} height={size + 4} /></span>;
  const I = GENERIC_ICONS[icon] ?? Cube;
  return <span className="appcard__icon" aria-hidden="true"><I size={size} weight="bold" /></span>;
}

const MAX_RASTER = 128;
const MAX_SVG = 64 * 1024;

/** An image file as a small data URI: rasters are scaled down to 128 px, SVG is kept as is (small ones only). */
async function readIcon(file: File): Promise<string> {
  if (file.type === 'image/svg+xml') {
    if (file.size > MAX_SVG) throw new Error('size');
    const text = await file.text();
    return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(text)))}`;
  }
  if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type)) throw new Error('type');
  const bmp = await createImageBitmap(file).catch(() => { throw new Error('decode'); });
  const k = Math.min(1, MAX_RASTER / Math.max(bmp.width, bmp.height));
  const w = Math.max(1, Math.round(bmp.width * k)), h = Math.max(1, Math.round(bmp.height * k));
  const canvas = document.createElement('canvas');
  canvas.width = w; canvas.height = h;
  canvas.getContext('2d')!.drawImage(bmp, 0, 0, w, h);
  return canvas.toDataURL('image/png');
}

type Mode = 'generic' | 'brand' | 'upload';
const modeOf = (v: string): Mode => (v.startsWith('brand:') ? 'brand' : v.startsWith('data:') ? 'upload' : 'generic');

/** Choose an app's icon: a generic glyph, a bundled logo, or an uploaded image. */
export function IconPicker({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const { t } = useI18n();
  const [mode, setMode] = useState<Mode>(modeOf(value));
  const [q, setQ] = useState('');
  const [err, setErr] = useState('');
  const file = useRef<HTMLInputElement>(null);
  const brands = useApi<BrandIcon[]>(mode === 'brand' || value.startsWith('brand:') ? '/portal/brand-icons' : null);
  const current = value.startsWith('brand:') ? brands.data?.find((b) => `brand:${b.slug}` === value)?.url : undefined;
  const shown = (brands.data ?? []).filter((b) => `${b.slug} ${b.title}`.toLowerCase().includes(q.trim().toLowerCase()));

  const pick = async (f: File | undefined) => {
    if (!f) return;
    setErr('');
    try { onChange(await readIcon(f)); } catch (e) { setErr(t((e as Error).message === 'size' ? 'portal.icon.err.size' : (e as Error).message === 'decode' ? 'portal.icon.err.decode' : 'portal.icon.err.type')); }
    if (file.current) file.current.value = '';
  };

  return (
    <div className="stack" style={{ gap: 8 }}>
      <div className="row" style={{ flexWrap: 'nowrap', gap: 12 }}>
        <AppIcon icon={value} url={current} size={22} />
        <Tabs<Mode> tabs={[{ id: 'generic', label: t('portal.icon.generic') }, { id: 'brand', label: t('portal.icon.brand') }, { id: 'upload', label: t('portal.icon.upload') }]} value={mode} onChange={setMode} />
      </div>
      {mode === 'generic' && (
        <Select aria-label={t('portal.app.icon')} value={GENERIC_ICONS[value] ? value : 'app-window'} onChange={(e) => onChange(e.target.value)}>
          {Object.keys(GENERIC_ICONS).filter(Boolean).map((k) => <option key={k} value={k}>{k}</option>)}
        </Select>
      )}
      {mode === 'brand' && (
        <>
          <SearchInput value={q} onChange={setQ} placeholder={t('portal.icon.search')} />
          <div className="iconpick" role="group" aria-label={t('portal.icon.brand')}>
            {shown.map((b) => (
              <button type="button" key={b.slug} className="iconpick__item" title={b.title} aria-label={b.title} aria-pressed={value === `brand:${b.slug}`} onClick={() => onChange(`brand:${b.slug}`)}>
                <img src={b.url} alt="" width={28} height={28} />
              </button>
            ))}
            {brands.data && shown.length === 0 && <span className="muted">{t('portal.icon.none')}</span>}
          </div>
          <p className="field__hint">{t('portal.icon.brand.hint')}</p>
        </>
      )}
      {mode === 'upload' && (
        <div className="stack" style={{ gap: 8 }}>
          <div className="row">
            <input ref={file} type="file" accept="image/png,image/jpeg,image/webp,image/svg+xml" className="sr-only" id="icon-file" onChange={(e) => void pick(e.target.files?.[0])} />
            <Button type="button" icon={<UploadSimple size={14} weight="bold" />} onClick={() => file.current?.click()}>{t('portal.icon.upload.pick')}</Button>
            {value.startsWith('data:') && <Button type="button" variant="ghost" icon={<Trash size={14} weight="bold" />} onClick={() => { onChange('app-window'); setErr(''); }}>{t('portal.icon.upload.remove')}</Button>}
          </div>
          <p className="field__hint" role={err ? 'alert' : undefined} style={err ? { color: 'var(--danger)' } : undefined}>{err || t('portal.icon.upload.hint')}</p>
        </div>
      )}
    </div>
  );
}
