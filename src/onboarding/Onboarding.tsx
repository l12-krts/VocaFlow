import { AnimatePresence, motion } from 'motion/react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { Button, Segment, SegmentedControl, Select, SelectItem, Switch } from 'krts.ui';
import { AccuracySlider } from '../components/AccuracySlider';
import { HotkeyRecorder, Keys } from '../components/Hotkey';
import { Info } from '../components/Info';
import { LevelMeter } from '../components/LevelMeter';
import { ModelStatus } from '../components/ModelStatus';
import { Chip } from '../components/Tags';
import { Logo, Wordmark } from '../components/Wordmark';
import { api } from '../lib/ipc';
import { LANGUAGES, languageName } from '../lib/languages';
import { useModels } from '../lib/useSettings';
import { usePrefersReducedMotion } from '../lib/usePrefersReducedMotion';
import type { MicDevice, Permissions, Platform, Settings } from '../lib/types';

type Update = (p: Partial<Settings>) => Promise<string | null>;

const STEPS = ['welcome', 'mic', 'shortcut', 'accuracy', 'formatting', 'try'] as const;
type Step = (typeof STEPS)[number];

export function Onboarding({ s, update, platform }: { s: Settings; update: Update; platform: Platform }) {
  const [i, setI] = useState(0);
  const [dir, setDir] = useState(1);
  const reduced = usePrefersReducedMotion();
  const step: Step = STEPS[i];
  const go = (d: number) => {
    setDir(d);
    setI((x) => Math.min(STEPS.length - 1, Math.max(0, x + d)));
  };
  const mac = platform.os === 'macos';

  return (
    <div className="flex h-full flex-col" data-tauri-drag-region>
      <div className={`flex items-center justify-center ${mac ? 'h-14 pt-4' : 'h-12'}`} data-tauri-drag-region>
        {i > 0 && <Dots count={STEPS.length - 1} active={i - 1} />}
      </div>
      <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden px-10">
        <AnimatePresence mode="wait" custom={dir} initial={false}>
          <motion.div
            key={step}
            custom={dir}
            className="w-full max-w-[440px]"
            initial={reduced ? { opacity: 0 } : { opacity: 0, x: dir * 24, filter: 'blur(3px)' }}
            animate={{ opacity: 1, x: 0, filter: 'blur(0px)', transition: { duration: 0.28, ease: [0.23, 1, 0.32, 1] } }}
            exit={reduced ? { opacity: 0 } : { opacity: 0, x: dir * -16, filter: 'blur(3px)', transition: { duration: 0.16, ease: [0.23, 1, 0.32, 1] } }}
          >
            {step === 'welcome' && <Welcome />}
            {step === 'mic' && <MicStep s={s} update={update} platform={platform} />}
            {step === 'shortcut' && <ShortcutStep s={s} update={update} />}
            {step === 'accuracy' && <AccuracyStep s={s} update={update} />}
            {step === 'formatting' && <FormattingStep s={s} update={update} />}
            {step === 'try' && <TryStep s={s} />}
          </motion.div>
        </AnimatePresence>
      </div>
      <div className="flex h-20 items-center justify-between px-8">
        <div>
          {i > 0 && (
            <Button variant="ghost" size="sm" onPress={() => go(-1)}>
              Back
            </Button>
          )}
        </div>
        {step === 'try' ? (
          <Button variant="primary" size="sm" className="min-w-28" onPress={() => update({ onboarded: true })}>
            Done
          </Button>
        ) : (
          <Button variant="primary" size="sm" className="min-w-28" onPress={() => go(1)}>
            {i === 0 ? 'Get started' : 'Continue'}
          </Button>
        )}
      </div>
    </div>
  );
}

function Dots({ count, active }: { count: number; active: number }) {
  return (
    <div className="flex items-center gap-1.5" aria-label={`Step ${active + 1} of ${count}`}>
      {Array.from({ length: count }, (_, n) => (
        <span
          key={n}
          className="h-1.5 rounded-full transition-[width,background-color] duration-300 ease-[cubic-bezier(0.23,1,0.32,1)]"
          style={{ width: n === active ? 18 : 6, background: n <= active ? 'var(--krts-primary)' : 'var(--krts-line)' }}
        />
      ))}
    </div>
  );
}

function Heading({ title, sub }: { title: string; sub?: ReactNode }) {
  return (
    <div className="mb-7 text-center">
      <h1 className="text-[24px] font-semibold tracking-[-0.02em] text-fg">{title}</h1>
      {sub && <p className="mt-1.5 text-[14px] text-fg-muted">{sub}</p>}
    </div>
  );
}

function Welcome() {
  return (
    <div className="flex flex-col items-center text-center">
      <div className="vf-hero-mark mb-7 flex size-[88px] items-center justify-center rounded-[24px]">
        <Logo size={38} className="text-white" />
      </div>
      <Wordmark size={40} className="text-fg" />
      <p className="mt-3 text-[15px] text-fg-muted">Speak naturally. It types, anywhere.</p>
    </div>
  );
}

function MicStep({ s, update, platform }: { s: Settings; update: Update; platform: Platform }) {
  const [mics, setMics] = useState<MicDevice[]>([]);
  const [perm, setPerm] = useState<Permissions | null>(null);
  useEffect(() => {
    api.mics().then(setMics);
    if (platform.os !== 'macos') return;
    const poll = () => api.permissions().then(setPerm);
    poll();
    // macOS grants arrive while we're in System Settings; poll gently.
    const t = setInterval(poll, 1200);
    return () => clearInterval(t);
  }, [platform.os]);

  return (
    <>
      <Heading title="Microphone" sub="Say something to check the level" />
      <div className="flex flex-col gap-4">
        <Select
          aria-label="Microphone"
          size="sm"
          selectedKey={s.micId ?? 'default'}
          onSelectionChange={(k) => update({ micId: k === 'default' ? null : String(k) })}
        >
          <SelectItem id="default">System default</SelectItem>
          {mics
            .filter((m) => !m.isDefault)
            .map((m) => (
              <SelectItem key={m.id} id={m.id}>
                {m.name}
              </SelectItem>
            ))}
        </Select>
        <div className="flex justify-center py-2">
          <LevelMeter />
        </div>
        {platform.os === 'macos' && perm && (
          <div className="mt-2 divide-y divide-line-subtle rounded-xl border border-line-subtle bg-bg px-4">
            <PermRow label="Microphone" info="Hears you while dictating" granted={perm.microphone === 'granted'} onGrant={() => api.requestPermission('microphone')} />
            <PermRow label="Accessibility" info="Pastes text, reads context" granted={perm.accessibility === 'granted'} onGrant={() => api.requestPermission('accessibility')} />
          </div>
        )}
        {platform.os === 'linux' && platform.wayland && (
          <p className="text-center text-[12.5px] text-fg-subtle">
            Wayland: install <code className="rounded bg-surface px-1">wtype</code> or <code className="rounded bg-surface px-1">ydotool</code> for pasting
          </p>
        )}
      </div>
    </>
  );
}

function PermRow({ label, info, granted, onGrant }: { label: string; info: string; granted: boolean; onGrant: () => void }) {
  return (
    <div className="flex h-[52px] items-center justify-between">
      <div className="flex items-center gap-1.5">
        <span className="text-[14px] font-medium text-fg">{label}</span>
        <Info>{info}</Info>
      </div>
      {granted ? (
        <span className="flex items-center gap-1.5 text-[13px] text-success">
          <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden>
            <path d="M3.5 8.5l3 3 6-7" stroke="currentColor" strokeWidth="1.8" fill="none" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          Allowed
        </span>
      ) : (
        <Button size="xs" variant="secondary" onPress={onGrant}>
          Allow
        </Button>
      )}
    </div>
  );
}

function ShortcutStep({ s, update }: { s: Settings; update: Update }) {
  const presets = ['Alt+D', 'F9'];
  const custom = !presets.includes(s.hotkey);
  return (
    <>
      <Heading title="Your shortcut" />
      <div className="flex flex-col items-center gap-6">
        <SegmentedControl
          aria-label="Shortcut"
          selectedKeys={[custom ? 'custom' : s.hotkey]}
          onSelectionChange={(k) => {
            const v = String([...k][0] ?? '');
            if (v && v !== 'custom') update({ hotkey: v });
          }}
        >
          <Segment id="Alt+D">
            <Keys accel="Alt+D" size="sm" />
          </Segment>
          <Segment id="F9">
            <Keys accel="F9" size="sm" />
          </Segment>
          <Segment id="custom">Custom</Segment>
        </SegmentedControl>
        <HotkeyRecorder value={s.hotkey} onChange={(hotkey) => update({ hotkey })} size="lg" />
        <div className="grid w-full grid-cols-2 gap-3">
          <Gesture title="Hold" sub="Talk, then let go" kind="hold" />
          <Gesture title="Tap" sub="Hands-free, tap to stop" kind="tap" />
        </div>
      </div>
    </>
  );
}

/** A tiny looping key illustration: held vs tapped. Rare screen, can delight. */
function Gesture({ title, sub, kind }: { title: string; sub: string; kind: 'hold' | 'tap' }) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-xl border border-line-subtle bg-bg px-4 py-4">
      <div className="relative flex h-10 items-end justify-center">
        <span className={`vf-key vf-key-${kind} block h-7 w-10 rounded-[7px] border border-line bg-surface`} />
        <span className={`vf-keywave vf-keywave-${kind}`} aria-hidden />
      </div>
      <div className="text-center">
        <div className="text-[13.5px] font-medium text-fg">{title}</div>
        <div className="text-[12.5px] text-fg-subtle">{sub}</div>
      </div>
    </div>
  );
}

function AccuracyStep({ s, update }: { s: Settings; update: Update }) {
  const models = useModels();
  const available = useMemo(() => LANGUAGES.filter((l) => !s.preferredLanguages.includes(l.code)), [s.preferredLanguages]);
  return (
    <>
      <Heading title="Accuracy" sub="Change it any time in Settings" />
      <div className="rounded-xl border border-line-subtle bg-bg px-5 py-4">
        <AccuracySlider value={s.quality} onChange={(quality) => update({ quality })} />
        <ModelStatus status={models?.speech} />
      </div>
      <div className="mt-5 flex items-center gap-1.5">
        <span className="text-[14px] font-medium text-fg">Languages you speak</span>
        <Info>Auto-detect picks from these</Info>
      </div>
      <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
        {s.preferredLanguages.map((code) => (
          <Chip key={code} onRemove={() => update({ preferredLanguages: s.preferredLanguages.filter((c) => c !== code) })}>
            {languageName(code)}
          </Chip>
        ))}
        <Select
          aria-label="Add language"
          size="sm"
          placeholder="Add"
          className="w-28 [&_button]:h-7 [&_button]:text-[13px]"
          selectedKey={null}
          onSelectionChange={(k) => k && update({ preferredLanguages: [...s.preferredLanguages, String(k)] })}
        >
          {available.map((l) => (
            <SelectItem key={l.code} id={l.code}>
              {l.name}
            </SelectItem>
          ))}
        </Select>
      </div>
    </>
  );
}

function FormattingStep({ s, update }: { s: Settings; update: Update }) {
  return (
    <>
      <Heading title="Smart formatting" sub="Cleans up and fits the app you're in" />
      <div className="flex items-center justify-between rounded-xl border border-line-subtle bg-bg px-4 py-3.5">
        <div className="flex items-center gap-1.5">
          <span className="text-[14px] font-medium text-fg">Context-aware formatting</span>
          <Info>Runs privately on this device</Info>
        </div>
        <Switch aria-label="Context-aware formatting" size="sm" isSelected={s.formatting} onChange={(formatting) => update({ formatting })} />
      </div>
      <div className={`mt-4 grid grid-cols-3 gap-2 transition-opacity duration-200 ${s.formatting ? '' : 'opacity-40'}`}>
        {[
          ['Mail', 'Email'],
          ['Terminal', 'Commit'],
          ['X', 'Post'],
        ].map(([where, what]) => (
          <div key={what} className="rounded-xl border border-line-subtle bg-surface/50 px-3 py-3 text-center">
            <div className="text-[12px] text-fg-subtle">{where}</div>
            <div className="vf-gradient-text text-[14px] font-medium">{what}</div>
          </div>
        ))}
      </div>
    </>
  );
}

function TryStep({ s }: { s: Settings }) {
  return (
    <>
      <Heading
        title="Try it"
        sub={
          <span className="inline-flex items-center gap-1.5">
            Hold <Keys accel={s.hotkey} size="sm" /> and say something
          </span>
        }
      />
      <textarea
        autoFocus
        placeholder="Your words land here"
        className="h-36 w-full resize-none rounded-xl border border-line bg-bg p-4 text-[14.5px] leading-relaxed text-fg outline-none transition-[border-color,box-shadow] duration-150 placeholder:text-fg-subtle focus:border-primary focus:shadow-[0_0_0_3px_rgb(19_92_245/0.12)]"
      />
    </>
  );
}
