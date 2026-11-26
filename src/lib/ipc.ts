import type { HistoryItem, MicDevice, ModelsState, Permissions, Platform, Settings } from './types';

/**
 * Thin typed layer over Tauri IPC. In a plain browser (vite dev without the
 * shell, or UI screenshots) it falls back to an in-memory mock so every
 * screen can be designed and tested on its own.
 */
export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

type Unlisten = () => void;

interface Backend {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten>;
}

let backendPromise: Promise<Backend> | null = null;

function backend(): Promise<Backend> {
  if (!backendPromise) {
    backendPromise = inTauri
      ? Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]).then(([core, ev]) => ({
          invoke: (cmd, args) => core.invoke(cmd, args),
          listen: async (event, cb) => ev.listen(event, (e) => cb(e.payload as never)),
        }))
      : import('./mock').then((m) => m.mockBackend);
  }
  return backendPromise;
}

const call = async <T>(cmd: string, args?: Record<string, unknown>) => (await backend()).invoke<T>(cmd, args);

export function on<T>(event: string, cb: (payload: T) => void): Unlisten {
  let un: Unlisten | null = null;
  let dead = false;
  backend()
    .then((b) => b.listen<T>(event, cb))
    .then((u) => (dead ? u() : (un = u)));
  return () => {
    dead = true;
    un?.();
  };
}

export const api = {
  platform: () => call<Platform>('platform'),
  getSettings: () => call<Settings>('get_settings'),
  setSettings: (patch: Partial<Settings>) => call<Settings>('set_settings', { patch }),
  models: () => call<ModelsState>('models_state'),
  retryDownload: () => call<void>('retry_download'),
  mics: () => call<MicDevice[]>('list_mics'),
  /** Start/stop streaming input level for the meter in settings/onboarding. */
  meter: (on: boolean) => call<void>('mic_meter', { on }),
  history: () => call<HistoryItem[]>('history'),
  clearHistory: () => call<void>('clear_history'),
  deleteHistory: (id: string) => call<void>('delete_history', { id }),
  copyText: (text: string) => call<void>('copy_text', { text }),
  permissions: () => call<Permissions>('permissions'),
  requestPermission: (kind: 'microphone' | 'accessibility') => call<void>('request_permission', { kind }),
  /** Suspend the global hotkey while the recorder captures a new one. */
  captureHotkey: (on: boolean) => call<void>('capture_hotkey', { on }),
  /** Try the formatter on sample text (onboarding preview). */
  tryDictation: () => call<void>('try_dictation'),
  pillReady: () => call<void>('pill_ready'),
  hideMain: () => call<void>('hide_main'),
  quit: () => call<void>('quit'),
};

export const events = {
  settings: 'settings://changed',
  pill: 'pill://state',
  level: 'audio://level',
  models: 'models://state',
  history: 'history://changed',
  insert: 'fx://insert',
  nav: 'main://navigate',
} as const;
