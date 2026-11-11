'use client';
import { CaretDown, Cube, ShieldCheck } from '@phosphor-icons/react';
import { useI18n } from '@/lib/i18n';
import { Button, Menu, MenuItem } from './ui';

/** What an update request takes: the operating system, the Docker containers, or both. */
export type Scope = 'system' | 'docker' | 'all';

/**
 * The button that starts updates, with a choice of what to update. With both kinds pending it opens a menu (system,
 * Docker, or both, each with its count); with only one kind pending it is a plain button that says which.
 * Nothing at all when nothing is pending.
 */
export function UpdateMenu({ system, docker, disabled, small, onPick }: { system: number; docker: number; disabled?: boolean; small?: boolean; onPick: (s: Scope) => void }) {
  const { t } = useI18n();
  if (system + docker === 0) return null;
  const size = small ? ' btn--sm' : '';
  if (docker === 0) return <Button variant="primary" small={small} icon={<ShieldCheck size={small ? 14 : 16} weight="bold" />} disabled={disabled} onClick={() => onPick('system')}>{t('upd.system', { n: system })}</Button>;
  if (system === 0) return <Button variant="primary" small={small} icon={<Cube size={small ? 14 : 16} weight="bold" />} disabled={disabled} onClick={() => onPick('docker')}>{t('upd.docker', { n: docker })}</Button>;
  return (
    <Menu label={t('upd.menu')} triggerClass={`btn btn--primary${size}${disabled ? ' is-disabled' : ''}`} trigger={<span className="row row--tight" style={{ flexWrap: 'nowrap' }}>{t('upd.menu')}<CaretDown size={small ? 12 : 14} weight="bold" /></span>}>
      {(close) => (
        <>
          <MenuItem icon={<ShieldCheck size={16} weight="bold" />} disabled={disabled} onClick={() => { close(); onPick('system'); }}>{t('upd.item.system', { n: system })}</MenuItem>
          <MenuItem icon={<Cube size={16} weight="bold" />} disabled={disabled} onClick={() => { close(); onPick('docker'); }}>{t('upd.item.docker', { n: docker })}</MenuItem>
          <MenuItem disabled={disabled} onClick={() => { close(); onPick('all'); }}>{t('upd.item.all', { n: system + docker })}</MenuItem>
        </>
      )}
    </Menu>
  );
}
