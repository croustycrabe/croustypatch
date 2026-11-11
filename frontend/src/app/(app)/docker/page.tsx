'use client';
import { DockerList } from '@/components/DockerList';
import { PageHeader } from '@/components/ui';
import { useI18n } from '@/lib/i18n';

export default function Page() {
  const { t } = useI18n();
  return (
    <>
      <PageHeader title={t('docker.title')} subtitle={t('docker.subtitle')} />
      <DockerList />
    </>
  );
}
