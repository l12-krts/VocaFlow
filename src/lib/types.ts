export type Quality = 'low' | 'medium' | 'high' | 'ultra';
export type ThemePref = 'system' | 'light' | 'dark';

export interface Settings {
  onboarded: boolean;
  hotkey: string;
  quality: Quality;
  /** "auto" or an ISO 639-1 code. */
  language: string;
  /** Languages auto-detect is limited to. Empty means any. */
  preferredLanguages: string[];
  formatting: boolean;
  theme: ThemePref;
  sounds: boolean;
  /** null = system default input. */
  micId: string | null;
  launchAtLogin: boolean;
  /** App identifiers that get typed instead of pasted. */
  typingApps: string[];
  dictionary: string[];
}

export type Phase = 'idle' | 'listening' | 'transcribing' | 'formatting' | 'error';

export interface PillState {
  phase: Phase;
  /** Hands-free (tap) vs. push-to-talk (hold). */
  handsFree: boolean;
  /** Formatting target shown as "Writing <format>…", e.g. "email". */
  format?: string | null;
  message?: string | null;
}

export type ModelStatus =
  | { state: 'ready' }
  | { state: 'missing' }
  | { state: 'downloading'; received: number; total: number }
  | { state: 'error'; message: string };

export interface ModelsState {
  /** Speech model for the selected quality. */
  speech: ModelStatus;
  /** Formatter model (only relevant when formatting is on). */
  formatter: ModelStatus;
}

export interface MicDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface HistoryItem {
  id: string;
  text: string;
  raw: string;
  app: string | null;
  format: string | null;
  at: number;
  durationMs: number;
}

export interface Platform {
  os: 'macos' | 'windows' | 'linux';
  /** Linux only: wayland sessions need portal permission. */
  wayland: boolean;
}

export interface Permissions {
  microphone: 'granted' | 'denied' | 'unknown';
  /** macOS Accessibility (paste + context). Always "granted" elsewhere. */
  accessibility: 'granted' | 'denied' | 'unknown';
}
