import { useRef, useState, type ReactNode } from 'react';
import { Button as AriaButton } from 'react-aria-components';

function Chip({ children, onRemove }: { children: ReactNode; onRemove: () => void }) {
  return (
    <span className="vf-chip inline-flex h-7 items-center gap-1 rounded-[7px] border border-line bg-surface pr-0.5 pl-2.5 text-[13px] text-fg">
      {children}
      <AriaButton
        aria-label={`Remove ${typeof children === 'string' ? children : ''}`}
        onPress={onRemove}
        className="inline-flex size-6 items-center justify-center rounded-[5px] text-fg-subtle outline-none transition-colors duration-150 hover:bg-surface-hover hover:text-fg focus-visible:outline-2 focus-visible:outline-focus"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M2 2l6 6M8 2l-6 6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      </AriaButton>
    </span>
  );
}

/** Free-form tags: Enter or comma adds, Backspace on empty removes the last. */
export function TagInput({
  values,
  onChange,
  placeholder,
  label,
}: {
  values: string[];
  onChange: (v: string[]) => void;
  placeholder: string;
  label: string;
}) {
  const [draft, setDraft] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const commit = () => {
    const parts = draft
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean)
      .filter((p) => !values.some((v) => v.toLowerCase() === p.toLowerCase()));
    if (parts.length) onChange([...values, ...parts]);
    setDraft('');
  };
  return (
    <div
      className="flex min-h-10 w-full flex-wrap items-center gap-1.5 rounded-[10px] border border-line bg-bg p-1.5 transition-[border-color,box-shadow] duration-150 focus-within:border-primary focus-within:shadow-[0_0_0_3px_rgb(19_92_245/0.12)]"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          e.preventDefault();
          input.current?.focus();
        }
      }}
    >
      {values.map((v) => (
        <Chip key={v} onRemove={() => onChange(values.filter((x) => x !== v))}>
          {v}
        </Chip>
      ))}
      <input
        ref={input}
        value={draft}
        aria-label={label}
        placeholder={values.length ? '' : placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ',') {
            e.preventDefault();
            commit();
          } else if (e.key === 'Backspace' && !draft && values.length) {
            onChange(values.slice(0, -1));
          }
        }}
        className="h-7 min-w-24 flex-1 bg-transparent px-1.5 text-[13.5px] text-fg outline-none placeholder:text-fg-subtle"
      />
    </div>
  );
}

export { Chip };
