'use client';
import { CompliancePage } from '@/components/Compliance';
import { PageHeader } from '@/components/ui';
import { useI18n } from '@/lib/i18n';

export default function Page() {
  const { t } = useI18n();
  return (
    <>
      <PageHeader title={t('comp.page.title')} subtitle={t('comp.page.subtitle')} />
      <CompliancePage />
    </>
  );
}
