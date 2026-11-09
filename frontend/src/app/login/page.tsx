'use client';
import { useRouter } from 'next/navigation';
import { useEffect, useState } from 'react';
import { AuthLayout } from '@/components/Shell';
import { Button, Field, Notice, TextInput } from '@/components/ui';
import { ApiError } from '@/lib/api';
import { useAuth } from '@/lib/auth';
import { useI18n } from '@/lib/i18n';

export default function LoginPage() {
  const { t } = useI18n();
  const { me, booting, login } = useAuth();
  const router = useRouter();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [failure, setFailure] = useState<'invalid' | 'locked' | null>(null);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    if (!booting && me) router.replace(me.must_change_password ? '/password/' : '/');
  }, [booting, me, router]);

  return (
    <AuthLayout>
      <div>
        <h1>{t('login.title')}</h1>
        <p className="muted" style={{ marginTop: 6 }}>{t('login.subtitle')}</p>
      </div>
      <form
        className="stack"
        onSubmit={async (e) => {
          e.preventDefault();
          setFailure(null);
          setPending(true);
          try {
            const m = await login(username.trim(), password);
            router.replace(m.must_change_password ? '/password/' : '/');
          } catch (err) {
            setFailure(err instanceof ApiError && err.status === 429 ? 'locked' : 'invalid');
          } finally {
            setPending(false);
          }
        }}
      >
        <Field label={t('login.username')}>{(id) => <TextInput id={id} value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" autoFocus required />}</Field>
        <Field label={t('login.password')}>{(id) => <TextInput id={id} type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" required />}</Field>
        {failure && <Notice kind="error">{failure === 'locked' ? t('login.locked') : t('login.invalid')}</Notice>}
        <Button variant="primary" type="submit" disabled={pending || !username || !password} style={{ width: '100%', height: 40 }}>{t('login.submit')}</Button>
      </form>
    </AuthLayout>
  );
}
