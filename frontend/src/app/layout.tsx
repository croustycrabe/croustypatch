import type { Metadata, Viewport } from 'next';
import type { ReactNode } from 'react';
import '@fontsource/rubik-one/400.css';
import '@fontsource/rubik/400.css';
import '@fontsource/rubik/500.css';
import '@fontsource/rubik/700.css';
import '@fontsource/roboto-mono/400.css';
import '@fontsource/roboto-mono/500.css';
import '@fontsource/roboto-mono/700.css';
import '@/styles/tokens.css';
import '@/styles/bundle.css';
import '@/styles/flat.css';
import { AuthProvider } from '@/lib/auth';
import { I18nProvider } from '@/lib/i18n';
import { ToastProvider } from '@/lib/toast';

export const metadata: Metadata = {
  title: 'CroustyPatch',
  description: 'Patch management · Croustylab',
  icons: { icon: '/brand/app-icon.svg' },
};
export const viewport: Viewport = { width: 'device-width', initialScale: 1 };

const THEME_BOOT = "try{var t=localStorage.getItem('cp.theme');if(t!=='light'&&t!=='dark'){t=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'}document.documentElement.dataset.theme=t}catch(e){document.documentElement.dataset.theme='light'}";

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="fr" suppressHydrationWarning>
      <head>
        {/* Theme before first paint: the saved choice, else the system preference (no flash of the wrong theme). */}
        <script dangerouslySetInnerHTML={{ __html: THEME_BOOT }} />
      </head>
      <body>
        <I18nProvider>
          <AuthProvider>
            <ToastProvider>{children}</ToastProvider>
          </AuthProvider>
        </I18nProvider>
      </body>
    </html>
  );
}
