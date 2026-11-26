import type { ReactNode } from 'react';
import { Info } from './Info';

/** A settings row: label (+ optional hint) on the left, control on the right. */
export function Row({
  label,
  info,
  children,
  stack = false,
  id,
  aside,
}: {
  label: ReactNode;
  info?: string;
  children: ReactNode;
  /** Put the control under the label (wide controls). */
  stack?: boolean;
  id?: string;
  /** Quiet text on the label line (stacked rows). */
  aside?: ReactNode;
}) {
  return (
    <div
      className={stack ? 'flex flex-col gap-3 py-3.5' : 'flex min-h-[52px] items-center justify-between gap-6 py-2.5'}
      id={id}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <span className="text-[14px] font-medium text-fg">{label}</span>
        {info && <Info>{info}</Info>}
        {aside && <span className="ml-auto truncate pl-4 text-[12.5px] text-fg-subtle">{aside}</span>}
      </div>
      <div className={stack ? 'w-full' : 'flex shrink-0 items-center'}>{children}</div>
    </div>
  );
}

export function Group({ children }: { children: ReactNode }) {
  return (
    <div className="divide-y divide-line-subtle rounded-xl border border-line-subtle bg-bg px-4 shadow-[0_1px_2px_rgb(0_0_0/0.03)]">
      {children}
    </div>
  );
}

export function PageTitle({ children, right }: { children: ReactNode; right?: ReactNode }) {
  return (
    <div className="mb-4 flex h-8 items-center justify-between">
      <h1 className="text-[19px] font-semibold tracking-[-0.015em] text-fg">{children}</h1>
      {right}
    </div>
  );
}
