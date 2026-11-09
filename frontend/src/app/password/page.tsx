'use client';
import { useRouter } from 'next/navigation';
import { useEffect, useState } from 'react';
import { AuthLayout } from '@/components/Shell';
import { Button, Field, Notice, TextInput } from '@/components/ui';
import { api } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useAction } from '@/lib/hooks';
import { useI18n } from '@/lib/i18n';

export default function PasswordPage() {
  const { t } = useI18n();
  const { me, booting, refresh, logout } = useAuth();
  const router = useRouter();
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const [again, setAgain] = useState('');
  const { pending, error, run } = useAction();
  const [local, setLocal] = useState<string | null>(null);

  useEffect(() => {
    if (!booting && !me) router.replace('/login/');
  }, [booting, me, router]);

  const submit = async () => {
    setLocal(null);
    if (next.length < 12) return setLocal(t('password.short'));
    if (next !== again) return setLocal(t('password.mismatch'));
    const ok = await run(async () => {
      await api('POST', '/auth/password', { current_password: current, new_password: next });
      return true;
    });
    if (ok) {
      await refresh();
      router.replace('/');
    }
  };

  return (
    <AuthLayout>
      <div>
        <h1>{t('password.title')}</h1>
        {me?.must_change_password && <p className="muted" style={{ marginTop: 6 }}>{t('password.forced')}</p>}
      </div>
      <form className="stack" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
        <Field label={t('password.current')}>{(id) => <TextInput id={id} type="password" value={current} onChange={(e) => setCurrent(e.target.value)} autoComplete="current-password" required />}</Field>
        <Field label={t('password.new')} hint={t('password.short')}>{(id) => <TextInput id={id} type="password" value={next} onChange={(e) => setNext(e.target.value)} autoComplete="new-password" minLength={12} required />}</Field>
        <Field label={t('password.confirm')}>{(id) => <TextInput id={id} type="password" value={again} onChange={(e) => setAgain(e.target.value)} autoComplete="new-password" required />}</Field>
        {(local || error) && <Notice kind="error">{local ?? error}</Notice>}
        <div className="row">
          <Button variant="primary" type="submit" disabled={pending}>{t('password.submit')}</Button>
          <Button variant="ghost" onClick={() => (me?.must_change_password ? void logout().then(() => router.replace('/login/')) : router.back())}>{me?.must_change_password ? t('nav.logout') : t('common.cancel')}</Button>
        </div>
      </form>
    </AuthLayout>
  );
}
