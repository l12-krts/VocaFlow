import { useEffect, useState } from 'react';
import { api, events, on } from './ipc';
import type { ThemePref } from './types';

const media = typeof window !== 'undefined' ? window.matchMedia('(prefers-color-scheme: dark)') : null;

export function resolveTheme(pref: ThemePref): 'light' | 'dark' {
  if (pref === 'system') return media?.matches ? 'dark' : 'light';
  return pref;
}

function apply(pref: ThemePref) {
  const t = resolveTheme(pref);
  document.documentElement.dataset.theme = t;
  document.documentElement.classList.toggle('dark', t === 'dark');
  document.documentElement.style.colorScheme = t;
}

/** Keeps <html data-theme> in sync with the setting and the OS. Returns the resolved theme. */
export function useTheme() {
  const [pref, setPref] = useState<ThemePref>('system');
  const [resolved, setResolved] = useState<'light' | 'dark'>(() => resolveTheme('system'));

  useEffect(() => {
    api.getSettings().then((s) => setPref(s.theme));
    return on<{ theme: ThemePref }>(events.settings, (s) => setPref(s.theme));
  }, []);

  useEffect(() => {
    const update = () => {
      apply(pref);
      setResolved(resolveTheme(pref));
    };
    update();
    media?.addEventListener('change', update);
    return () => media?.removeEventListener('change', update);
  }, [pref]);

  return resolved;
}
