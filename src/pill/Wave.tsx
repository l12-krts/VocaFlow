import { useEffect, useRef } from 'react';
import { events, on } from '../lib/ipc';

/**
 * The Figma "Centered tapered wave": a 256×36 ribbon whose top edge bulges
 * up a little left of centre and whose bottom edge bulges down a little
 * right of centre, tapering to points at both ends.
 *
 * Figma path (amplitude 17.86, peaks at x=116.36 / x=139.64):
 *   M0,18 C39.56,18 62.84,-1.85 116.36,0.14 C169.89,2.12 196.65,18 256,18
 *         C208.29,18 190.84,37.85 139.64,35.86 C88.44,33.88 55.85,18 0,18 Z
 *
 * We keep those control-point ratios and drive three numbers per frame:
 * the top/bottom amplitude (voice level) and the peak positions (a slow
 * drift so the ribbon feels alive even during pauses).
 */
const W = 256;
const H = 36;
const MID = H / 2;
const FIGMA_A = 17.86;

function ribbon(top: number, bottom: number, pt: number, pb: number) {
  // Scale vertical offsets from the Figma geometry; shift x around the peaks.
  const t = top / FIGMA_A;
  const b = bottom / FIGMA_A;
  const dt = pt - 116.36;
  const db = pb - 139.64;
  const f = (n: number) => n.toFixed(2);
  return (
    `M0,${MID}` +
    `C39.56,${MID} ${f(62.84 + dt)},${f(MID - 19.85 * t)} ${f(pt)},${f(MID - 17.86 * t)}` +
    `C${f(169.89 + dt)},${f(MID - 15.88 * t)} 196.65,${MID} ${W},${MID}` +
    `C208.29,${MID} ${f(190.84 + db)},${f(MID + 19.85 * b)} ${f(pb)},${f(MID + 17.86 * b)}` +
    `C${f(88.44 + db)},${f(MID + 15.88 * b)} 55.85,${MID} 0,${MID}Z`
  );
}

export function Wave({ active, reduced }: { active: boolean; reduced: boolean }) {
  const main = useRef<SVGPathElement>(null);
  const ghost = useRef<SVGPathElement>(null);
  const target = useRef(0);

  useEffect(() => on<number>(events.level, (l) => (target.current = l)), []);

  useEffect(() => {
    if (!active) return;
    let raf = 0;
    let level = 0;
    let last = performance.now();
    let clock = 0;
    const frame = (now: number) => {
      const dt = Math.max(0, Math.min(0.05, (now - last) / 1000));
      last = now;
      clock += dt;
      // Fast attack, slow release: speech onsets feel immediate, the ribbon
      // relaxes instead of snapping flat between words.
      const k = target.current > level ? 1 - Math.exp(-dt * 28) : 1 - Math.exp(-dt * 6);
      level += (target.current - level) * k;

      const voice = Math.pow(Math.min(1, Math.max(0, level)), 0.65);
      const breathe = reduced ? 0 : Math.sin(clock * 2.1) * 0.9;
      const amp = 3.2 + breathe + voice * 14.6;
      const drift = reduced ? 0 : 1;
      const pt = 116.36 + Math.sin(clock * 1.35) * 16 * drift;
      const pb = 139.64 + Math.sin(clock * 1.05 + 1.3) * 16 * drift;
      const skew = 1 + Math.sin(clock * 3.1) * 0.08 * voice;

      main.current?.setAttribute('d', ribbon(amp * skew, amp / skew, pt, pb));
      ghost.current?.setAttribute(
        'd',
        ribbon(amp * 0.72 / skew, amp * 0.72 * skew, 256 - pb + Math.sin(clock * 0.8) * 10, 256 - pt),
      );
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [active, reduced]);

  return (
    <svg width={W} height={H} viewBox={`0 0 ${W} ${H}`} className="block overflow-visible" aria-hidden>
      <defs>
        <linearGradient id="vf-wave" x1="0" x2={W} y1="0" y2="0" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="rgb(18 92 245)" stopOpacity="0.76" />
          <stop offset="1" stopColor="rgb(69 207 255)" stopOpacity="0.7" />
        </linearGradient>
        {/* Figma: inner shadow 0 4 5.2 rgb(157 203 255 / .88) */}
        <filter id="vf-inner" x="-5%" y="-60%" width="110%" height="220%">
          <feFlood floodColor="rgb(157 203 255)" floodOpacity="0.88" />
          <feComposite in2="SourceAlpha" operator="out" />
          <feOffset dy="4" />
          <feGaussianBlur stdDeviation="2.6" />
          <feComposite in2="SourceAlpha" operator="in" result="shadow" />
          <feMerge>
            <feMergeNode in="SourceGraphic" />
            <feMergeNode in="shadow" />
          </feMerge>
        </filter>
      </defs>
      <path ref={ghost} d={ribbon(2.3, 2.3, 139.64, 116.36)} fill="url(#vf-wave)" opacity="0.32" />
      <path ref={main} d={ribbon(3.2, 3.2, 116.36, 139.64)} fill="url(#vf-wave)" opacity="0.85" filter="url(#vf-inner)" />
    </svg>
  );
}
