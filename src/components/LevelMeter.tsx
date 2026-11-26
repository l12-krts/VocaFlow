import { useEffect, useRef } from 'react';
import { api, events, on } from '../lib/ipc';

/**
 * Live input level as 24 small bars. Values go straight to the DOM (no
 * React renders at 30 fps); bars fall slower than they rise like a VU meter.
 */
export function LevelMeter({ active = true }: { active?: boolean }) {
  const bars = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!active) return;
    api.meter(true);
    let level = 0;
    let target = 0;
    const off = on<number>(events.level, (l) => (target = l));
    let raf = 0;
    const tick = () => {
      level += (target - level) * (target > level ? 0.5 : 0.12);
      const el = bars.current;
      if (el) {
        const n = el.children.length;
        const lit = Math.round(Math.min(1, level * 1.15) * n);
        for (let i = 0; i < n; i++) (el.children[i] as HTMLElement).dataset.on = i < lit ? '1' : '';
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => {
      off();
      cancelAnimationFrame(raf);
      api.meter(false);
    };
  }, [active]);

  return (
    <div ref={bars} className="flex h-4 items-center gap-[3px]" aria-hidden>
      {Array.from({ length: 24 }, (_, i) => (
        <span
          key={i}
          className="h-full w-[4px] rounded-full bg-surface-pressed transition-colors duration-75 data-[on='1']:bg-primary"
        />
      ))}
    </div>
  );
}
