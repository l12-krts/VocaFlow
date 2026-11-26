import { useEffect, useState } from 'react';
import { Button as AriaButton } from 'react-aria-components';
import { Button } from 'krts.ui';
import { PageTitle } from '../components/Row';
import { api, events, on } from '../lib/ipc';
import type { HistoryItem } from '../lib/types';

function when(at: number) {
  const d = new Date(at);
  const diff = Date.now() - at;
  if (diff < 60_000) return 'Just now';
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`;
  const today = new Date().toDateString() === d.toDateString();
  const time = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  if (today) return time;
  const yesterday = new Date(Date.now() - 86_400_000).toDateString() === d.toDateString();
  return yesterday ? `Yesterday ${time}` : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

export function HistoryPage() {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [copied, setCopied] = useState<string | null>(null);
  useEffect(() => {
    api.history().then(setItems);
    return on<HistoryItem[]>(events.history, setItems);
  }, []);

  const copy = (it: HistoryItem) => {
    api.copyText(it.text);
    setCopied(it.id);
    setTimeout(() => setCopied((c) => (c === it.id ? null : c)), 1200);
  };

  return (
    <>
      <PageTitle
        right={
          items.length > 0 && (
            <Button size="xs" variant="ghost" onPress={() => api.clearHistory()}>
              Clear
            </Button>
          )
        }
      >
        History
      </PageTitle>
      {items.length === 0 ? (
        <div className="flex h-64 flex-col items-center justify-center gap-1 text-center">
          <span className="text-[14px] font-medium text-fg">Nothing yet</span>
          <span className="text-[13px] text-fg-subtle">Your last 50 dictations show up here</span>
        </div>
      ) : (
        <ul className="flex flex-col gap-2">
          {items.map((it) => (
            <li
              key={it.id}
              className="group relative rounded-xl border border-line-subtle bg-bg px-4 py-3 transition-[border-color] duration-150 hover:border-line"
            >
              <p className="line-clamp-3 pr-16 text-[13.5px] leading-relaxed whitespace-pre-line text-fg" data-selectable>
                {it.text}
              </p>
              <div className="mt-1.5 flex items-center gap-1.5 text-[12px] text-fg-subtle">
                <span>{when(it.at)}</span>
                {it.app && (
                  <>
                    <span aria-hidden>·</span>
                    <span>{it.app}</span>
                  </>
                )}
                {it.format && it.format !== 'text' && (
                  <>
                    <span aria-hidden>·</span>
                    <span className="capitalize">{it.format}</span>
                  </>
                )}
              </div>
              <div className="absolute top-2.5 right-2.5 flex gap-0.5 opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100">
                <IconButton label={copied === it.id ? 'Copied' : 'Copy'} onPress={() => copy(it)}>
                  {copied === it.id ? (
                    <path d="M3.5 8.5l3 3 6-7" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round" strokeLinejoin="round" />
                  ) : (
                    <>
                      <rect x="5.5" y="5.5" width="8" height="8" rx="1.6" stroke="currentColor" strokeWidth="1.3" fill="none" />
                      <path d="M10.5 3.5v-.4A1.6 1.6 0 0 0 8.9 1.5H4.1A1.6 1.6 0 0 0 2.5 3.1v4.8a1.6 1.6 0 0 0 1.6 1.6h.4" stroke="currentColor" strokeWidth="1.3" fill="none" />
                    </>
                  )}
                </IconButton>
                <IconButton label="Delete" onPress={() => api.deleteHistory(it.id)}>
                  <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.4a1 1 0 0 0 1 .9h3.8a1 1 0 0 0 1-.9l.6-8.4" stroke="currentColor" strokeWidth="1.3" fill="none" strokeLinecap="round" strokeLinejoin="round" />
                </IconButton>
              </div>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function IconButton({ label, onPress, children }: { label: string; onPress: () => void; children: React.ReactNode }) {
  return (
    <AriaButton
      aria-label={label}
      onPress={onPress}
      className="inline-flex size-7 items-center justify-center rounded-[7px] text-fg-muted outline-none transition-[background-color,color,transform] duration-150 ease-out hover:bg-surface-hover hover:text-fg pressed:scale-[0.96] focus-visible:outline-2 focus-visible:outline-focus"
    >
      <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden>
        {children}
      </svg>
    </AriaButton>
  );
}
