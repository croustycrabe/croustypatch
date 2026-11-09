'use client';
import type { ReactNode } from 'react';
import { Shell } from '@/components/Shell';

/** The sidebar, topbar and command palette live here, so they stay mounted while pages change. */
export default function AppLayout({ children }: { children: ReactNode }) {
  return <Shell>{children}</Shell>;
}
