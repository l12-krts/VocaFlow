import '@fontsource-variable/inter';
import './pill.css';
import { StrictMode, useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { api, events, inTauri, on } from '../lib/ipc';
import { useTheme } from '../lib/theme';
import type { PillState, Platform } from '../lib/types';
import { Pill } from './Pill';
import { playCue } from './sounds';

function PillApp() {
  useTheme();
  const [state, setState] = useState<PillState>({ phase: 'idle', handsFree: false });
  const [platform, setPlatform] = useState<Platform>({ os: 'windows', wayland: false });
  const [sounds, setSounds] = useState(true);
  const preview = !inTauri;

  useEffect(() => {
    api.platform().then(setPlatform);
    api.getSettings().then((s) => setSounds(s.sounds));
    const offs = [
      on<{ sounds: boolean }>(events.settings, (s) => setSounds(s.sounds)),
      on<PillState>(events.pill, (next) => setState(next)),
    ];
    api.pillReady();
    if (preview && new URLSearchParams(location.search).has('bg')) document.body.classList.add('preview-bg');
    if (preview && new URLSearchParams(location.search).has('demo')) {
      import('../lib/mock').then((m) => m.runPillDemo());
    }
    return () => offs.forEach((f) => f());
  }, [preview]);

  // Audio cues follow phase edges, not every state message.
  const [prev, setPrev] = useState(state.phase);
  if (prev !== state.phase) {
    if (sounds) {
      if (state.phase === 'listening') playCue('start');
      else if (prev === 'listening' && state.phase !== 'error') playCue('stop');
      else if (state.phase === 'error' && prev === 'listening') playCue('cancel');
    }
    setPrev(state.phase);
  }

  const onShape = useCallback(
    (width: number) => {
      if (inTauri && platform.os === 'macos') {
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('pill_shape', {
            width: Math.max(width, 72),
            visible: width > 0,
            reduced: matchMedia('(prefers-reduced-motion: reduce)').matches,
          }),
        );
      }
    },
    [platform.os],
  );

  return (
    <div data-preview={preview || undefined} className="contents">
      <Pill state={state} platform={platform} onShape={onShape} />
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <PillApp />
  </StrictMode>,
);
