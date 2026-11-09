'use client';
import { useCallback, useEffect, useRef, useState } from 'react';
import { api, ApiError } from './api';

interface State<T> {
  data: T | undefined;
  error: ApiError | undefined;
  loading: boolean;
}

/** GET a path; optionally poll. A path of `null` skips the request (conditional fetching). */
export function useApi<T>(path: string | null, opts?: { poll?: number }) {
  const [state, setState] = useState<State<T>>({ data: undefined, error: undefined, loading: path !== null });
  const seq = useRef(0);

  const load = useCallback(
    async (silent: boolean) => {
      if (path === null) return;
      const mine = ++seq.current;
      if (!silent) setState((s) => ({ ...s, loading: true }));
      try {
        const data = await api<T>('GET', path);
        if (mine === seq.current) setState({ data, error: undefined, loading: false });
      } catch (e) {
        // Ignore answers that were overtaken by a newer request.
        if (mine === seq.current) setState((s) => ({ data: silent ? s.data : undefined, error: e instanceof ApiError ? e : new ApiError(0, 'network', String(e)), loading: false }));
      }
    },
    [path],
  );

  useEffect(() => {
    void load(false);
    if (!opts?.poll || path === null) return () => void seq.current++;
    const id = setInterval(() => {
      if (document.visibilityState === 'visible') void load(true);
    }, opts.poll);
    return () => {
      clearInterval(id);
      seq.current++;
    };
  }, [load, opts?.poll, path]);

  const reload = useCallback(() => load(true), [load]);
  const setData = useCallback((fn: (d: T | undefined) => T | undefined) => setState((s) => ({ ...s, data: fn(s.data) })), []);
  return { ...state, reload, setData };
}

/** Run an async action with a pending flag and an error message. */
export function useAction() {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = useCallback(async <R,>(fn: () => Promise<R>): Promise<R | undefined> => {
    setPending(true);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
      return undefined;
    } finally {
      setPending(false);
    }
  }, []);
  return { pending, error, run, clearError: () => setError(null) };
}
