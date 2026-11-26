import { AnimatePresence, motion, type Transition } from 'motion/react';
import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { usePrefersReducedMotion } from '../lib/usePrefersReducedMotion';
import type { PillState, Platform } from '../lib/types';
import { Wave } from './Wave';

/** Figma geometry, in px. */
const H = 72;
const LISTEN_W = 256;
const LOADER_W = 128;
const PAD_L = 26;
const LOADER_SPAN = 18 + 11 + 18 + 11 + 18; // dot, bar, dot
const TEXT_GAP = 15;
const PAD_R = 22;

/**
 * Size morphs are a spring on every platform except macOS, where the native
 * glass layer behind the webview is animated by Core Animation and can only
 * follow a timing curve. There we use the same curve on both sides so the
 * glass and the tint never drift apart mid-morph.
 */
export const MAC_MORPH = { duration: 0.42, ease: [0.32, 0.72, 0, 1] as const };

export function Pill({ state, platform, onShape }: { state: PillState; platform: Platform; onShape?: (w: number) => void }) {
  const reduced = usePrefersReducedMotion();
  const label = state.phase === 'formatting' ? `Writing ${state.format ?? 'text'}…` : null;
  const message = state.phase === 'error' ? state.message ?? 'Something went wrong' : null;

  // Measure the label offscreen so the pill can morph straight to its final
  // width instead of growing in two steps.
  const measure = useRef<HTMLSpanElement>(null);
  const [textW, setTextW] = useState(0);
  const text = label ?? message;
  useLayoutEffect(() => {
    if (measure.current) setTextW(Math.ceil(measure.current.getBoundingClientRect().width));
  }, [text]);

  const width = useMemo(() => {
    switch (state.phase) {
      case 'listening':
        return LISTEN_W;
      case 'transcribing':
        return LOADER_W;
      case 'formatting':
        return PAD_L + LOADER_SPAN + TEXT_GAP + textW + PAD_R;
      case 'error':
        return Math.max(LOADER_W, 28 + textW + 28);
      default:
        return LOADER_W;
    }
  }, [state.phase, textW]);

  useLayoutEffect(() => onShape?.(width), [width, onShape]);

  const visible = state.phase !== 'idle';
  const mac = platform.os === 'macos';

  const morph: Transition = reduced
    ? { duration: 0 }
    : mac
      ? MAC_MORPH
      : { type: 'spring', duration: 0.45, bounce: 0.16 };

  // Content swaps crossfade with a touch of blur so the two states read as
  // one object changing, not two objects overlapping.
  const swap = reduced
    ? { initial: { opacity: 0 }, animate: { opacity: 1 }, exit: { opacity: 0 } }
    : {
        initial: { opacity: 0, filter: 'blur(4px)', scale: 0.96 },
        animate: { opacity: 1, filter: 'blur(0px)', scale: 1 },
        exit: { opacity: 0, filter: 'blur(4px)', scale: 0.96 },
      };

  const content =
    state.phase === 'listening' ? 'wave' : state.phase === 'error' ? 'message' : 'loader';

  return (
    <>
      <span ref={measure} className="pill-label pointer-events-none invisible absolute whitespace-nowrap" aria-hidden>
        {text}
      </span>
      <AnimatePresence>
        {visible && (
          <motion.div
            key="pill"
            role="status"
            aria-live="polite"
            aria-label={
              state.phase === 'listening' ? 'Listening' : state.phase === 'transcribing' ? 'Transcribing' : text ?? ''
            }
            className="pill relative overflow-hidden"
            data-os={platform.os}
            style={{ height: H, borderRadius: H / 2, originY: 1 }}
            initial={reduced ? { opacity: 0, width } : { opacity: 0, scale: 0.9, y: 8, width }}
            animate={{
              opacity: 1,
              scale: 1,
              y: 0,
              width,
              transition: {
                opacity: { duration: 0.18, ease: [0.23, 1, 0.32, 1] },
                scale: { duration: 0.22, ease: [0.23, 1, 0.32, 1] },
                y: { duration: 0.22, ease: [0.23, 1, 0.32, 1] },
                width: morph,
              },
            }}
            exit={
              reduced
                ? { opacity: 0, transition: { duration: 0.12 } }
                : { opacity: 0, scale: 0.94, y: 4, transition: { duration: 0.16, ease: [0.23, 1, 0.32, 1] } }
            }
          >
            <div className="pill-noise" aria-hidden />
            <AnimatePresence initial={false} mode="popLayout">
              {content === 'wave' && (
                <motion.div key="wave" className="absolute inset-0 flex items-center justify-center" {...swap} transition={{ duration: 0.2 }}>
                  <Wave active reduced={reduced} />
                </motion.div>
              )}
              {content === 'loader' && (
                <motion.div
                  key="loader"
                  className="absolute inset-y-0 left-0 flex items-center"
                  style={{ paddingLeft: state.phase === 'formatting' ? PAD_L : (LOADER_W - LOADER_SPAN) / 2 }}
                  {...swap}
                  transition={{ duration: 0.2 }}
                  layout="position"
                >
                  <Loader reduced={reduced} />
                  <AnimatePresence initial={false}>
                    {label && (
                      <motion.span
                        key={label}
                        className="pill-label pill-gradient-text whitespace-nowrap"
                        style={{ marginLeft: TEXT_GAP }}
                        initial={{ opacity: 0, x: -4 }}
                        animate={{ opacity: 1, x: 0, transition: { duration: 0.24, delay: 0.06, ease: [0.23, 1, 0.32, 1] } }}
                        exit={{ opacity: 0, transition: { duration: 0.1 } }}
                      >
                        {label}
                      </motion.span>
                    )}
                  </AnimatePresence>
                </motion.div>
              )}
              {content === 'message' && (
                <motion.div key="msg" className="absolute inset-0 flex items-center justify-center px-7" {...swap} transition={{ duration: 0.2 }}>
                  <span className="pill-label pill-message whitespace-nowrap">{message}</span>
                </motion.div>
              )}
            </AnimatePresence>
          </motion.div>
        )}
      </AnimatePresence>
    </>
  );
}

/** Dot, bar, dot: the bar's height travels left to right, Figma "Transcribing". */
function Loader({ reduced }: { reduced: boolean }) {
  return (
    <div className="flex items-center gap-[11px]" data-reduced={reduced || undefined}>
      <span className="pill-blob loader-blob" style={{ animationDelay: '0ms' }} />
      <span className="pill-blob loader-blob" style={{ animationDelay: '150ms' }} />
      <span className="pill-blob loader-blob" style={{ animationDelay: '300ms' }} />
    </div>
  );
}
