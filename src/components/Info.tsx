import { Button as AriaButton } from 'react-aria-components';
import { Tooltip, TooltipTrigger } from 'krts.ui';

/** The (i) affordance: a 4–5 word hint, never a paragraph. */
export function Info({ children }: { children: string }) {
  return (
    <TooltipTrigger delay={350} closeDelay={150}>
      <AriaButton
        aria-label={children}
        className="inline-flex size-[18px] shrink-0 items-center justify-center rounded-full text-fg-subtle outline-none transition-colors duration-150 ease-out hover:text-fg-muted focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus"
      >
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden>
          <circle cx="8" cy="8" r="6.6" stroke="currentColor" strokeWidth="1.3" />
          <path d="M8 7.2v3.6" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
          <circle cx="8" cy="5.1" r=".85" fill="currentColor" />
        </svg>
      </AriaButton>
      <Tooltip placement="top" className="bg-tip text-tip-fg text-[12.5px] px-2 py-1 rounded-md">
        {children}
      </Tooltip>
    </TooltipTrigger>
  );
}
