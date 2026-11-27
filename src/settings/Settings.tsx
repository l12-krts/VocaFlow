import { motion } from 'motion/react';
import { useEffect, useState, type ReactNode } from 'react';
import { Keys } from '../components/Hotkey';
import { Wordmark } from '../components/Wordmark';
import { events, on } from '../lib/ipc';
import { useModels } from '../lib/useSettings';
import type { Platform, Settings } from '../lib/types';
import { HistoryPage } from './History';
import { DictationPage, FormattingPage, GeneralPage } from './pages';

type Page = 'general' | 'dictation' | 'formatting' | 'history';

const NAV: { id: Page; label: string; icon: ReactNode }[] = [
  {
    id: 'general',
    label: 'General',
    icon: (
      <>
        <circle cx="8" cy="8" r="2.2" stroke="currentColor" strokeWidth="1.4" fill="none" />
        <path d="M8 1.8v1.6M8 12.6v1.6M14.2 8h-1.6M3.4 8H1.8M12.4 3.6l-1.1 1.1M4.7 11.3l-1.1 1.1M12.4 12.4l-1.1-1.1M4.7 4.7L3.6 3.6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      </>
    ),
  },
  {
    id: 'dictation',
    label: 'Dictation',
    icon: (
      <>
        <rect x="5.6" y="1.8" width="4.8" height="8" rx="2.4" stroke="currentColor" strokeWidth="1.4" fill="none" />
        <path d="M3.3 7.6a4.7 4.7 0 0 0 9.4 0M8 12.3v2" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" fill="none" />
      </>
    ),
  },
  {
    id: 'formatting',
    label: 'Formatting',
    icon: (
      <>
        <path d="M2.5 4h11M2.5 8h7M2.5 12h4.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        <path d="M12.2 9.3l.55 1.45 1.45.55-1.45.55-.55 1.45-.55-1.45-1.45-.55 1.45-.55z" fill="currentColor" />
      </>
    ),
  },
  {
    id: 'history',
    label: 'History',
    icon: (
      <>
        <circle cx="8" cy="8" r="6" stroke="currentColor" strokeWidth="1.4" fill="none" />
        <path d="M8 4.8V8l2.2 1.4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" fill="none" />
      </>
    ),
  },
];

export function SettingsView({
  s,
  update,
  platform,
}: {
  s: Settings;
  update: (p: Partial<Settings>) => Promise<string | null>;
  platform: Platform;
}) {
  const [page, setPage] = useState<Page>('general');
  const models = useModels();
  useEffect(() => on<string>(events.nav, (p) => NAV.some((n) => n.id === p) && setPage(p as Page)), []);
  const mac = platform.os === 'macos';

  return (
    <div className="flex h-full">
      <aside className="flex w-[208px] shrink-0 flex-col border-r border-line-subtle bg-sidebar px-3 pb-3" data-tauri-drag-region>
        <div className={`flex items-center px-2 ${mac ? 'h-[76px] pt-7' : 'h-16'}`} data-tauri-drag-region>
          <Wordmark size={19} className="pointer-events-none text-fg" />
        </div>
        <nav className="flex flex-col gap-0.5" aria-label="Settings">
          {NAV.map((n) => {
            const active = n.id === page;
            return (
              <button
                key={n.id}
                onClick={() => setPage(n.id)}
                aria-current={active ? 'page' : undefined}
                className={`relative flex h-9 items-center gap-2.5 rounded-[8px] px-2.5 text-left text-[14px] outline-none transition-colors duration-150 focus-visible:outline-2 focus-visible:outline-focus ${active ? 'text-fg' : 'text-fg-muted hover:text-fg'}`}
              >
                {active && (
                  <motion.span
                    layoutId="nav-active"
                    className="absolute inset-0 rounded-[8px] bg-bg shadow-[0_0_0_1px_var(--krts-line-subtle),0_1px_2px_rgb(0_0_0/0.04)]"
                    transition={{ duration: 0.35, ease: [0.16, 1, 0.3, 1] }}
                  />
                )}
                <svg width="16" height="16" viewBox="0 0 16 16" className="relative shrink-0" aria-hidden>
                  {n.icon}
                </svg>
                <span className="relative">{n.label}</span>
              </button>
            );
          })}
        </nav>
        <div className="mt-auto flex items-center gap-2 px-2.5 text-[12px] text-fg-subtle">
          <Keys accel={s.hotkey} size="sm" />
          <span>to dictate</span>
        </div>
      </aside>
      <main className="min-w-0 flex-1 overflow-y-auto">
        <div className={`mx-auto max-w-[560px] px-8 pb-10 ${mac ? 'pt-9' : 'pt-7'}`} key={page}>
          <div className="vf-fade">
            {page === 'general' && <GeneralPage s={s} update={update} />}
            {page === 'dictation' && <DictationPage s={s} update={update} models={models} />}
            {page === 'formatting' && <FormattingPage s={s} update={update} models={models} />}
            {page === 'history' && <HistoryPage />}
          </div>
        </div>
      </main>
    </div>
  );
}
