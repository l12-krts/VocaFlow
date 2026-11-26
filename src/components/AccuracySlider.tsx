import { Slider, SliderThumb, SliderTrack } from 'react-aria-components';
import type { Quality } from '../lib/types';

export const TIERS: { id: Quality; label: string }[] = [
  { id: 'low', label: 'Low' },
  { id: 'medium', label: 'Medium' },
  { id: 'high', label: 'High' },
  { id: 'ultra', label: 'Ultra' },
];

/**
 * Four detents. Ultra turns the fill into slowly flowing liquid (the pill's
 * blue/cyan plus a violet) and gives the thumb a soft halo. The effect
 * fades in over the plain fill so moving in and out of Ultra never pops.
 */
export function AccuracySlider({ value, onChange }: { value: Quality; onChange: (q: Quality) => void }) {
  const index = TIERS.findIndex((t) => t.id === value);
  const ultra = value === 'ultra';
  return (
    <Slider
      aria-label="Accuracy"
      minValue={0}
      maxValue={3}
      step={1}
      value={index}
      onChange={(v) => onChange(TIERS[v as number].id)}
      className="group w-full select-none"
      data-ultra={ultra || undefined}
    >
      <SliderTrack className="relative h-8 w-full touch-none">
        {({ state }) => {
          const pct = state.getThumbPercent(0) * 100;
          const dragging = state.isThumbDragging(0);
          return (
            <>
              <div className="absolute inset-x-0 top-1/2 h-2 -translate-y-1/2 rounded-full bg-surface shadow-hairline" />
              {/* Detents */}
              {TIERS.map((t, i) => (
                <span
                  key={t.id}
                  aria-hidden
                  className="absolute top-1/2 size-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-line-strong/70 transition-opacity duration-200"
                  style={{ left: `${(i / 3) * 100}%`, opacity: i <= index ? 0 : 1 }}
                />
              ))}
              <div
                className={`vf-fill absolute top-1/2 left-0 h-2 -translate-y-1/2 rounded-full ${dragging ? '' : 'transition-[width] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)]'}`}
                style={{ width: `${pct}%` }}
              >
                <span className="vf-fill-solid" />
                <span className="vf-fill-liquid" aria-hidden />
                <span className="vf-fill-glow" aria-hidden />
              </div>
              <SliderThumb
                className={`vf-thumb top-1/2 size-6 rounded-full border border-surface bg-white outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus ${dragging ? '' : 'transition-[left] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)]'}`}
              >
                <span className="vf-thumb-halo" aria-hidden />
              </SliderThumb>
            </>
          );
        }}
      </SliderTrack>
      <div className="relative mt-1 h-5 text-[12.5px]">
        {TIERS.map((t, i) => (
          <span
            key={t.id}
            className={`absolute top-0 whitespace-nowrap transition-colors duration-150 ${i === index ? 'font-medium text-fg' : 'text-fg-subtle'} ${t.id === 'ultra' && ultra ? 'vf-ultra-label' : ''}`}
            style={{
              left: `${(i / 3) * 100}%`,
              transform: i === 0 ? 'none' : i === 3 ? 'translateX(-100%)' : 'translateX(-50%)',
            }}
          >
            {t.label}
          </span>
        ))}
      </div>
    </Slider>
  );
}
