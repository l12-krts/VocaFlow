import { useEffect, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { events, inTauri, on } from '../lib/ipc';

/**
 * The paste shimmer. The window sits over the line where text lands (24px
 * padding around it); a soft blue→cyan light sweeps left to right across
 * the inserted run and fades out. ~450ms, transform/opacity only, and a
 * plain fade under Reduce Motion.
 */
const PAD = 24;

function Shimmer() {
  const band = useRef<HTMLDivElement>(null);
  const line = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
    const play = ({ width, height }: { width: number; height: number }) => {
      const b = box.current;
      if (!b || !band.current || !line.current) return;
      b.style.width = `${width}px`;
      b.style.height = `${height}px`;
      const ease = 'cubic-bezier(0.23, 1, 0.32, 1)';
      if (reduced) {
        line.current.animate([{ opacity: 0 }, { opacity: 0.7 }, { opacity: 0 }], { duration: 500, easing: 'ease' });
        return;
      }
      band.current.animate(
        [
          { transform: `translateX(-60%)`, opacity: 0 },
          { opacity: 1, offset: 0.2 },
          { opacity: 0.9, offset: 0.75 },
          { transform: `translateX(${width}px)`, opacity: 0 },
        ],
        { duration: Math.min(620, 360 + width * 0.35), easing: ease },
      );
      line.current.animate(
        [
          { transform: 'scaleX(0)', opacity: 0.9 },
          { transform: 'scaleX(1)', opacity: 0.8, offset: 0.55 },
          { transform: 'scaleX(1)', opacity: 0 },
        ],
        { duration: Math.min(760, 460 + width * 0.35), easing: ease },
      );
    };
    const off = on(events.insert, play);
    if (!inTauri && new URLSearchParams(location.search).has('demo')) {
      const t = setInterval(() => play({ width: 320, height: 22 }), 1400);
      return () => (off(), clearInterval(t));
    }
    return off;
  }, []);

  return (
    <div ref={box} style={{ position: 'absolute', left: PAD, top: PAD, overflow: 'visible' }}>
      <div
        ref={band}
        style={{
          position: 'absolute',
          top: -4,
          bottom: -4,
          left: 0,
          width: 'max(60px, 30%)',
          opacity: 0,
          borderRadius: 8,
          background:
            'linear-gradient(90deg, rgb(69 207 255 / 0), rgb(69 207 255 / 0.28) 40%, rgb(120 170 255 / 0.36) 55%, rgb(18 92 245 / 0))',
          filter: 'blur(6px)',
          willChange: 'transform, opacity',
        }}
      />
      <div
        ref={line}
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          bottom: -3,
          height: 2,
          borderRadius: 2,
          opacity: 0,
          transformOrigin: 'left center',
          background: 'linear-gradient(90deg, rgb(18 92 245 / 0.7), rgb(69 207 255 / 0.7))',
          boxShadow: '0 0 8px rgb(69 207 255 / 0.6)',
          willChange: 'transform, opacity',
        }}
      />
    </div>
  );
}

document.documentElement.style.background = 'transparent';
document.body.style.cssText = 'margin:0;background:transparent;overflow:hidden';
createRoot(document.getElementById('root')!).render(<Shimmer />);
