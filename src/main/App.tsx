import { useEffect, useRef, useState } from 'react';
import { api, inTauri } from '../lib/ipc';
import { useTheme } from '../lib/theme';
import { useSettings } from '../lib/useSettings';
import type { Platform } from '../lib/types';
import { Onboarding } from '../onboarding/Onboarding';
import { SettingsView } from '../settings/Settings';

export function App() {
  const theme = useTheme();
  const [s, update] = useSettings();
  const [platform, setPlatform] = useState<Platform | null>(null);
  const shown = useRef(false);

  useEffect(() => {
    api.platform().then((p) => {
      setPlatform(p);
      document.documentElement.dataset.os = p.os;
    });
  }, []);

  // Tell the shell we've painted (no white flash), and keep the mac
  // glass appearance in step with the app theme.
  useEffect(() => {
    if (!s || !platform || !inTauri) return;
    requestAnimationFrame(() =>
      import('@tauri-apps/api/core').then(({ invoke }) => {
        invoke('main_ready', { dark: theme === 'dark' });
        shown.current = true;
      }),
    );
  }, [s, platform, theme]);

  if (!s || !platform) return null;
  return s.onboarded ? <SettingsView s={s} update={update} platform={platform} /> : <Onboarding s={s} update={update} platform={platform} />;
}
