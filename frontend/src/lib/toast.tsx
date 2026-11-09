'use client';
import { CheckCircle, Info, X, XCircle } from '@phosphor-icons/react';
import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from 'react';

type Kind = 'success' | 'error' | 'info';
interface Item { id: number; kind: Kind; text: string }
interface Api { success: (t: string) => void; error: (t: string) => void; info: (t: string) => void }

const Ctx = createContext<Api | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Item[]>([]);
  const next = useRef(1);
  const dismiss = useCallback((id: number) => setItems((l) => l.filter((x) => x.id !== id)), []);
  const push = useCallback((kind: Kind, text: string) => {
    const id = next.current++;
    setItems((l) => [...l.slice(-2), { id, kind, text }]); // at most three
    // design rule: success and info leave after 5 s; an error stays until it is closed
    if (kind !== 'error') setTimeout(() => dismiss(id), 5000);
  }, [dismiss]);
  const api = useMemo<Api>(() => ({ success: (t) => push('success', t), error: (t) => push('error', t), info: (t) => push('info', t) }), [push]);

  return (
    <Ctx.Provider value={api}>
      {children}
      <div className="toasts" aria-live="polite">
        {items.map((i) => (
          <div key={i.id} className={`toast toast--${i.kind}`} role={i.kind === 'error' ? 'alert' : 'status'}>
            {i.kind === 'error' ? <XCircle size={20} weight="bold" className="icon" /> : i.kind === 'success' ? <CheckCircle size={20} weight="bold" className="icon" /> : <Info size={20} weight="bold" className="icon" />}
            <span>{i.text}</span>
            <button type="button" aria-label="×" onClick={() => dismiss(i.id)}><X size={14} weight="bold" /></button>
          </div>
        ))}
      </div>
    </Ctx.Provider>
  );
}

export function useToast(): Api {
  const c = useContext(Ctx);
  if (!c) throw new Error('useToast outside ToastProvider');
  return c;
}
