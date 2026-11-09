'use client';
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { allows, api, ApiError, setUnauthorizedHandler } from './api';
import type { Me, Modules } from './types';

interface AuthCtx {
  me: Me | null;
  /** true until the first /auth/me answer */
  booting: boolean;
  modules: Modules | null;
  can: (perm: string) => boolean;
  login: (username: string, password: string) => Promise<Me>;
  logout: () => Promise<void>;
  refresh: () => Promise<void>;
}

const Auth = createContext<AuthCtx | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null>(null);
  const [booting, setBooting] = useState(true);
  const [modules, setModules] = useState<Modules | null>(null);

  const loadModules = useCallback(async () => {
    try {
      setModules(await api<Modules>('GET', '/modules'));
    } catch {
      setModules(null);
    }
  }, []);

  const refresh = useCallback(async () => {
    try {
      const m = await api<Me>('GET', '/auth/me');
      setMe(m);
      if (!m.must_change_password) void loadModules();
    } catch (e) {
      if (!(e instanceof ApiError) || e.status !== 401) console.error(e);
      setMe(null);
    } finally {
      setBooting(false);
    }
  }, [loadModules]);

  useEffect(() => {
    setUnauthorizedHandler(() => setMe(null));
    void refresh();
    return () => setUnauthorizedHandler(null);
  }, [refresh]);

  const login = useCallback(
    async (username: string, password: string) => {
      const m = await api<Me>('POST', '/auth/login', { username, password });
      setMe(m);
      if (!m.must_change_password) void loadModules();
      return m;
    },
    [loadModules],
  );

  const logout = useCallback(async () => {
    try {
      await api('POST', '/auth/logout');
    } finally {
      setMe(null);
      setModules(null);
    }
  }, []);

  const value = useMemo<AuthCtx>(() => ({ me, booting, modules, can: (p) => (me ? allows(me.permissions, p) : false), login, logout, refresh }), [me, booting, modules, login, logout, refresh]);
  return <Auth.Provider value={value}>{children}</Auth.Provider>;
}

export function useAuth(): AuthCtx {
  const c = useContext(Auth);
  if (!c) throw new Error('useAuth outside AuthProvider');
  return c;
}
