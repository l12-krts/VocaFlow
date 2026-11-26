import type { HistoryItem, ModelsState, PillState, Settings } from './types';

/** Browser-only stand-in for the Rust backend. Not shipped in the app path. */
type Cb = (p: unknown) => void;
const subs = new Map<string, Set<Cb>>();
const emit = (event: string, payload: unknown) => subs.get(event)?.forEach((cb) => cb(payload));

const params = new URLSearchParams(location.search);

let settings: Settings = {
  onboarded: params.get('onboarded') !== '0',
  hotkey: 'Alt+D',
  quality: 'medium',
  language: 'auto',
  preferredLanguages: ['en', 'el'],
  formatting: true,
  theme: (params.get('theme') as Settings['theme']) ?? 'system',
  sounds: true,
  micId: null,
  launchAtLogin: true,
  typingApps: [],
  dictionary: ['VocaFlow', 'Tauri', 'krts'],
};

let models: ModelsState = { speech: { state: 'ready' }, formatter: { state: 'ready' } };

const now = Date.now();
let history: HistoryItem[] = [
  {
    id: '1',
    text: 'Hi Maria,\n\nThanks for sending the draft over. I went through it this morning and the structure works well. Could we move the pricing section before the timeline?\n\nBest,\nAlex',
    raw: 'hi maria thanks for sending the draft over um i went through it this morning and the structure works well could we move the pricing section before the timeline best alex',
    app: 'Mail',
    format: 'email',
    at: now - 1000 * 60 * 4,
    durationMs: 9400,
  },
  {
    id: '2',
    text: 'fix(pill): keep the wave centered when the window is scaled',
    raw: 'fix pill keep the wave centered when the window is scaled',
    app: 'Terminal',
    format: 'commit',
    at: now - 1000 * 60 * 38,
    durationMs: 3100,
  },
  {
    id: '3',
    text: 'Shipping the new dictation pill today. It feels so much calmer than the old one.',
    raw: 'shipping the new dictation pill today it feels so much calmer than the old one',
    app: 'Arc',
    format: 'post',
    at: now - 1000 * 60 * 60 * 26,
    durationMs: 4200,
  },
];

function startDownloadSim() {
  const total = { low: 57, medium: 190, high: 574, ultra: 1080 }[settings.quality] * 1024 * 1024;
  let received = 0;
  const t = setInterval(() => {
    received = Math.min(total, received + total / 40);
    models = { ...models, speech: received >= total ? { state: 'ready' } : { state: 'downloading', received, total } };
    emit('models://state', models);
    if (received >= total) clearInterval(t);
  }, 80);
}

let meterTimer: ReturnType<typeof setInterval> | null = null;

export const mockBackend = {
  async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    switch (cmd) {
      case 'platform':
        return { os: (params.get('os') as never) ?? 'windows', wayland: false } as T;
      case 'get_settings':
        return settings as T;
      case 'set_settings': {
        const patch = args!.patch as Partial<Settings>;
        const qualityChanged = patch.quality && patch.quality !== settings.quality;
        settings = { ...settings, ...patch };
        emit('settings://changed', settings);
        if (qualityChanged) startDownloadSim();
        return settings as T;
      }
      case 'models_state':
        return models as T;
      case 'list_mics':
        return [
          { id: 'default', name: 'MacBook Pro Microphone', isDefault: true },
          { id: 'usb', name: 'Shure MV7+', isDefault: false },
          { id: 'airpods', name: 'AirPods Pro', isDefault: false },
        ] as T;
      case 'mic_meter':
        if (meterTimer) clearInterval(meterTimer);
        meterTimer = null;
        if (args!.on) {
          let t = 0;
          meterTimer = setInterval(() => {
            t += 1;
            const speech = Math.max(0, Math.sin(t / 6)) * (0.5 + 0.5 * Math.random());
            emit('audio://level', Math.min(1, speech * 0.9 + Math.random() * 0.04));
          }, 33);
        }
        return undefined as T;
      case 'history':
        return history as T;
      case 'clear_history':
        history = [];
        emit('history://changed', history);
        return undefined as T;
      case 'delete_history':
        history = history.filter((h) => h.id !== args!.id);
        emit('history://changed', history);
        return undefined as T;
      case 'permissions':
        return { microphone: 'granted', accessibility: params.get('ax') === '0' ? 'denied' : 'granted' } as T;
      default:
        return undefined as T;
    }
  },
  async listen<T>(event: string, cb: (payload: T) => void) {
    const set = subs.get(event) ?? new Set();
    set.add(cb as Cb);
    subs.set(event, set);
    return () => set.delete(cb as Cb);
  },
};

/** Drives the pill through a full dictation for design review: ?demo */
export function runPillDemo(loop = true) {
  const seq: [number, PillState][] = [
    [600, { phase: 'listening', handsFree: false }],
    [3600, { phase: 'transcribing', handsFree: false }],
    [1400, { phase: 'formatting', handsFree: false, format: 'email' }],
    [1600, { phase: 'idle', handsFree: false }],
  ];
  let level = 0;
  let t = 0;
  const lv = setInterval(() => {
    t += 1;
    const target = Math.max(0, Math.sin(t / 7) * 0.8 + Math.sin(t / 2.3) * 0.25) * (0.6 + Math.random() * 0.4);
    level += (target - level) * 0.35;
    emit('audio://level', Math.min(1, level));
  }, 33);
  let i = 0;
  const step = () => {
    const [delay, state] = seq[i % seq.length];
    setTimeout(() => {
      emit('pill://state', state);
      i += 1;
      if (loop || i < seq.length) step();
      else clearInterval(lv);
    }, delay);
  };
  step();
}

export function mockEmit(event: string, payload: unknown) {
  emit(event, payload);
}
