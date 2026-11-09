'use client';
import { AuthLayout } from '@/components/Shell';
import { LinkButton } from '@/components/ui';
import { useI18n } from '@/lib/i18n';

export default function NotFound() {
  const { t } = useI18n();
  return (
    <AuthLayout>
      <div>
        <h1>{t('notfound.title')}</h1>
        <p className="muted" style={{ marginTop: 6 }}>{t('notfound.text')}</p>
      </div>
      <img src="/brand/not-found.svg" alt="" width={280} style={{ maxWidth: '100%', height: 'auto' }} />
      <div><LinkButton href="/" variant="primary">{t('notfound.back')}</LinkButton></div>
    </AuthLayout>
  );
}
