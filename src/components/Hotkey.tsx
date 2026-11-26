import { useEffect, useRef, useState } from 'react';
import { Button as AriaButton } from 'react-aria-components';
import { api } from '../lib/ipc';

const isMac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform || navigator.userAgent);

const MAC: Record<string, string> = { Alt: '⌥', Ctrl: '⌃', Control: '⌃', Shift: '⇧', Super: '⌘', Cmd: '⌘', Command: '⌘' };
const PC: Record<string, string> = { Super: 'Win', Control: 'Ctrl' };

/** "Alt+D" → ["⌥", "D"] on macOS, ["Alt", "D"] elsewhere. */
export function hotkeyParts(accel: string): string[] {
  return accel.split('+').map((p) => (isMac ? MAC[p] : PC[p]) ?? p);
}

export function Keys({ accel, size = 'md' }: { accel: string; size?: 'sm' | 'md' | 'lg' }) {
  const cls =
    size === 'lg'
      ? 'min-w-11 h-11 px-3 text-[17px] rounded-[10px]'
      : size === 'sm'
        ? 'min-w-6 h-6 px-1.5 text-[12px] rounded-[6px]'
        : 'min-w-8 h-8 px-2 text-[13.5px] rounded-[7px]';
  return (
    <span className="inline-flex items-center gap-1">
      {hotkeyParts(accel).map((k, i) => (
        <kbd
          key={i}
          className={`${cls} inline-flex items-center justify-center border border-line bg-surface font-sans font-medium text-fg shadow-[inset_0_-1px_0_rgb(0_0_0/0.08)] dark:shadow-[inset_0_-1px_0_rgb(0_0_0/0.4)]`}
        >
          {k}
        </kbd>
      ))}
    </span>
  );
}

const MODS = ['Control', 'Alt', 'Shift', 'Meta'];

function codeToKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  const map: Record<string, string> = {
    Space: 'Space', Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
    Backslash: '\\', Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/',
    Insert: 'Insert', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', Pause: 'Pause',
  };
  return map[code] ?? null;
}

/**
 * Click, press the new combination, done. F-keys work alone; anything else
 * needs a modifier so typing a letter never triggers dictation.
 */
export function HotkeyRecorder({
  value,
  onChange,
  size = 'md',
}: {
  value: string;
  onChange: (accel: string) => Promise<string | null>;
  size?: 'md' | 'lg';
}) {
  const [recording, setRecording] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!recording) return;
    api.captureHotkey(true);
    const down = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        setRecording(false);
        return;
      }
      const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && (isMac ? 'Cmd' : 'Super')].filter(
        Boolean,
      ) as string[];
      if (MODS.includes(e.key)) {
        setPreview(mods.join('+') || null);
        return;
      }
      const key = codeToKey(e.code);
      if (!key) return;
      const isF = /^F\d+$/.test(key);
      if (!isF && mods.length === 0) {
        setError('Add a modifier key');
        return;
      }
      const accel = [...mods, key].join('+');
      setRecording(false);
      onChange(accel).then((err) => setError(err));
    };
    const up = (e: KeyboardEvent) => {
      const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && (isMac ? 'Cmd' : 'Super')].filter(Boolean);
      setPreview(mods.length ? (mods as string[]).join('+') : null);
    };
    const blur = () => setRecording(false);
    window.addEventListener('keydown', down, true);
    window.addEventListener('keyup', up, true);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('keydown', down, true);
      window.removeEventListener('keyup', up, true);
      window.removeEventListener('blur', blur);
      api.captureHotkey(false);
      setPreview(null);
    };
  }, [recording, onChange]);

  return (
    <div className="flex flex-col items-end gap-1.5">
      <AriaButton
        ref={ref}
        onPress={() => {
          setError(null);
          setRecording((r) => !r);
        }}
        aria-label={recording ? 'Press the new shortcut' : `Shortcut ${value}. Change`}
        data-recording={recording || undefined}
        className={`group relative inline-flex items-center justify-center rounded-[10px] border border-line bg-bg px-1.5 outline-none transition-[border-color,box-shadow] duration-150 ease-out hover:border-line-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus data-[recording]:border-primary data-[recording]:shadow-[0_0_0_3px_rgb(19_92_245/0.15)] ${size === 'lg' ? 'h-14 min-w-40' : 'h-10 min-w-28'}`}
      >
        {recording ? (
          preview ? (
            <Keys accel={preview} size={size === 'lg' ? 'lg' : 'md'} />
          ) : (
            <span className="px-2 text-[13.5px] text-fg-muted">Press keys</span>
          )
        ) : (
          <Keys accel={value} size={size === 'lg' ? 'lg' : 'md'} />
        )}
      </AriaButton>
      {error && <span className="text-[12px] text-danger">{error}</span>}
    </div>
  );
}
