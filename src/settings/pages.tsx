import { useEffect, useState } from 'react';
import { SegmentedControl, Segment, Select, SelectItem, Switch } from 'krts.ui';
import { AccuracySlider } from '../components/AccuracySlider';
import { HotkeyRecorder } from '../components/Hotkey';
import { LevelMeter } from '../components/LevelMeter';
import { ModelStatus } from '../components/ModelStatus';
import { Group, PageTitle, Row } from '../components/Row';
import { Chip, TagInput } from '../components/Tags';
import { api } from '../lib/ipc';
import { LANGUAGES, languageName } from '../lib/languages';
import type { MicDevice, ModelsState, Settings, ThemePref } from '../lib/types';

type Update = (p: Partial<Settings>) => Promise<string | null>;

export function GeneralPage({ s, update }: { s: Settings; update: Update }) {
  const [mics, setMics] = useState<MicDevice[]>([]);
  useEffect(() => {
    api.mics().then(setMics);
  }, []);
  return (
    <>
      <PageTitle>General</PageTitle>
      <Group>
        <Row label="Shortcut" info="Hold to talk, tap to toggle">
          <HotkeyRecorder value={s.hotkey} onChange={(hotkey) => update({ hotkey })} />
        </Row>
        <Row label="Microphone" stack>
          <div className="flex items-center gap-4">
            <Select
              aria-label="Microphone"
              className="min-w-0 flex-1"
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
            <LevelMeter />
          </div>
        </Row>
      </Group>

      <div className="h-5" />
      <Group>
        <Row label="Appearance">
          <SegmentedControl
            aria-label="Appearance"
            selectedKeys={[s.theme]}
            onSelectionChange={(k) => {
              const v = [...k][0] as ThemePref | undefined;
              if (v) update({ theme: v });
            }}
          >
            <Segment id="system">System</Segment>
            <Segment id="light">Light</Segment>
            <Segment id="dark">Dark</Segment>
          </SegmentedControl>
        </Row>
        <Row label="Sounds">
          <Switch aria-label="Sounds" size="sm" isSelected={s.sounds} onChange={(sounds) => update({ sounds })} />
        </Row>
        <Row label="Open at login">
          <Switch aria-label="Open at login" size="sm" isSelected={s.launchAtLogin} onChange={(launchAtLogin) => update({ launchAtLogin })} />
        </Row>
      </Group>
    </>
  );
}

export function DictationPage({ s, update, models }: { s: Settings; update: Update; models: ModelsState | null }) {
  const available = LANGUAGES.filter((l) => !s.preferredLanguages.includes(l.code));
  return (
    <>
      <PageTitle>Dictation</PageTitle>
      <Group>
        <Row label="Accuracy" info="Higher is slower, more precise" stack>
          <div className="px-1">
            <AccuracySlider value={s.quality} onChange={(quality) => update({ quality })} />
          </div>
          <ModelStatus status={models?.speech} />
        </Row>
      </Group>

      <div className="h-5" />
      <Group>
        <Row label="Language">
          <Select
            aria-label="Language"
            size="sm"
            className="w-48"
            selectedKey={s.language}
            onSelectionChange={(k) => update({ language: String(k) })}
          >
            {[{ code: 'auto', name: 'Auto-detect' }, ...LANGUAGES].map((l) => (
              <SelectItem key={l.code} id={l.code}>
                {l.name}
              </SelectItem>
            ))}
          </Select>
        </Row>
        {s.language === 'auto' && (
          <Row label="Preferred languages" info="Auto-detect picks from these" stack>
            <div className="flex flex-wrap items-center gap-1.5">
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
          </Row>
        )}
      </Group>

      <div className="h-5" />
      <Group>
        <Row label="Dictionary" info="Names and terms to spell" stack>
          <TagInput label="Dictionary" values={s.dictionary} onChange={(dictionary) => update({ dictionary })} placeholder="Add a word" />
        </Row>
        <Row label="Type instead of paste" info="For apps that block paste" stack>
          <TagInput label="Apps" values={s.typingApps} onChange={(typingApps) => update({ typingApps })} placeholder="App name" />
        </Row>
      </Group>
    </>
  );
}

export function FormattingPage({ s, update, models }: { s: Settings; update: Update; models: ModelsState | null }) {
  return (
    <>
      <PageTitle>Formatting</PageTitle>
      <Group>
        <Row label="Context-aware formatting" info="Shapes text for each app">
          <Switch aria-label="Context-aware formatting" size="sm" isSelected={s.formatting} onChange={(formatting) => update({ formatting })} />
        </Row>
        {s.formatting && models?.formatter && models.formatter.state !== 'ready' && (
          <div className="pb-3.5">
            <ModelStatus status={models.formatter} />
          </div>
        )}
      </Group>
      <FormatPreview on={s.formatting} />
    </>
  );
}

const SAMPLES = [
  { where: 'Mail', label: 'Email', raw: 'hi maria um thanks for the draft I think we should move pricing before the timeline best alex', out: 'Hi Maria,\n\nThanks for the draft. I think we should move pricing before the timeline.\n\nBest,\nAlex' },
  { where: 'Terminal', label: 'Commit', raw: 'fix the pill so the wave stays centered when the window scales', out: 'fix: keep the pill wave centered when the window scales' },
  { where: 'X', label: 'Post', raw: 'shipping the new dictation pill today uh it feels so much calmer than the old one', out: 'Shipping the new dictation pill today. It feels so much calmer than the old one.' },
];

/** Three before/after pairs; the selected one crossfades. */
function FormatPreview({ on }: { on: boolean }) {
  const [i, setI] = useState(0);
  const sample = SAMPLES[i];
  return (
    <div className={`mt-5 transition-opacity duration-200 ${on ? 'opacity-100' : 'opacity-45'}`}>
      <SegmentedControl aria-label="Example" selectedKeys={[String(i)]} onSelectionChange={(k) => setI(Number([...k][0] ?? 0))}>
        {SAMPLES.map((x, n) => (
          <Segment key={x.label} id={String(n)}>
            {x.label}
          </Segment>
        ))}
      </SegmentedControl>
      <div className="mt-3 grid grid-cols-2 gap-3">
        <div className="rounded-xl border border-line-subtle bg-surface/60 p-3.5">
          <div className="mb-2 text-[11.5px] font-medium tracking-wide text-fg-subtle uppercase">Spoken</div>
          <p key={`r${i}`} className="vf-fade text-[13.5px] leading-relaxed text-fg-muted italic">
            “{sample.raw}”
          </p>
        </div>
        <div className="rounded-xl border border-line-subtle bg-bg p-3.5">
          <div className="mb-2 flex items-center justify-between text-[11.5px] font-medium tracking-wide text-fg-subtle uppercase">
            <span>{sample.where}</span>
          </div>
          <p key={`o${i}`} className="vf-fade text-[13.5px] leading-relaxed whitespace-pre-line text-fg" data-selectable>
            {on ? sample.out : sample.raw.charAt(0).toUpperCase() + sample.raw.slice(1) + '.'}
          </p>
        </div>
      </div>
    </div>
  );
}
