import { AnimatePresence, motion, type Transition } from 'motion/react';
import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { usePrefersReducedMotion } from '../lib/usePrefersReducedMotion';
import type { PillState, Platform } from '../lib/types';
import { Wave } from './Wave';

/** Figma geometry, in px. */
const H = 72;
const DOT = H; // the closed shape: a circle
const LISTEN_W = 256;
const LOADER_W = 128;
const PAD_L = 26;
const LOADER_SPAN = 18 + 11 + 18 + 11 + 18; // dot, bar, dot
const TEXT_GAP = 15;
const PAD_R = 22;

const EASE_OUT = [0.23, 1, 0.32, 1] as const;
const DRAWER = [0.32, 0.72, 0, 1] as const;

/**
 * Choreography (Dynamic Island-like, one object the whole time):
 *
 *  open    a 72px circle rises and fades in, then stretches to its width
 *          (spring); the content blurs in once there's room for it.
 *  change  width springs to the new state while the old content blurs out
 *          and the new one blurs in; loader blobs pop in with a stagger.
 *  close   content fades first, the pill pulls back into a circle, then
 *          the circle shrinks away. Faster than opening, as exits should be.
 *
 * macOS uses timing curves instead of springs because the native glass
 * behind the webview is driven by Core Animation and must stay in step.
 */
export function Pill({ state, platform, onShape }: { state: PillState; platform: Platform; onShape?: (w: number) => void }) {
  const reduced = usePrefersReducedMotion();
  const mac = platform.os === 'macos';
  const label = state.phase === 'formatting' ? `Writing ${state.format ?? 'text'}…` : null;
  const message = state.phase === 'error' ? state.message ?? 'Something went wrong' : null;

  // Measure text offscreen so the pill morphs straight to its final width.
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
      case 'formatting':
        return PAD_L + LOADER_SPAN + TEXT_GAP + textW + PAD_R;
      case 'error':
        return Math.max(LOADER_W, 28 + textW + 28);
      default:
        return LOADER_W;
    }
  }, [state.phase, textW]);

  const visible = state.phase !== 'idle';
  useLayoutEffect(() => onShape?.(visible ? width : 0), [width, visible, onShape]);

  // The first frame after opening waits a beat before stretching, so the
  // circle reads as the origin of the pill.
  const openedAt = useRef(0);
  const [wasVisible, setWasVisible] = useState(false);
  if (visible !== wasVisible) {
    if (visible) openedAt.current = performance.now();
    setWasVisible(visible);
  }
  const justOpened = performance.now() - openedAt.current < 120;

  const widthT: Transition = reduced
    ? { duration: 0 }
    : mac
      ? { duration: justOpened ? 0.5 : 0.42, ease: DRAWER, delay: justOpened ? 0.05 : 0 }
      : { type: 'spring', duration: justOpened ? 0.55 : 0.45, bounce: justOpened ? 0.24 : 0.16, delay: justOpened ? 0.06 : 0 };

  const content = state.phase === 'listening' ? 'wave' : state.phase === 'error' ? 'message' : 'loader';
  const contentDelay = justOpened ? 0.14 : 0.06;

  const swap = reduced
    ? {
        initial: { opacity: 0 },
        animate: { opacity: 1, transition: { duration: 0.15 } },
        exit: { opacity: 0, transition: { duration: 0.1 } },
      }
    : {
        initial: { opacity: 0, scale: 0.92, filter: 'blur(4px)' },
        animate: { opacity: 1, scale: 1, filter: 'blur(0px)', transition: { duration: 0.24, delay: contentDelay, ease: EASE_OUT } },
        exit: { opacity: 0, scale: 0.94, filter: 'blur(4px)', transition: { duration: 0.12, ease: EASE_OUT } },
      };

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
            aria-label={state.phase === 'listening' ? 'Listening' : state.phase === 'transcribing' ? 'Transcribing' : text ?? ''}
            className="pill relative overflow-hidden"
            data-os={platform.os}
            style={{ height: H, borderRadius: H / 2 }}
            initial={reduced ? { opacity: 0, width } : { opacity: 0, scale: 0.8, y: 10, width: DOT }}
            animate={{
              opacity: 1,
              scale: 1,
              y: 0,
              width,
              transition: {
                opacity: { duration: 0.16, ease: EASE_OUT },
                scale: mac ? { duration: 0.3, ease: EASE_OUT } : { type: 'spring', duration: 0.4, bounce: 0.3 },
                y: mac ? { duration: 0.3, ease: EASE_OUT } : { type: 'spring', duration: 0.4, bounce: 0.2 },
                width: widthT,
              },
            }}
            exit={
              reduced
                ? { opacity: 0, transition: { duration: 0.12 } }
                : {
                    width: [null, DOT, DOT],
                    opacity: [1, 1, 0],
                    scale: [1, 1, 0.82],
                    y: [0, 0, 6],
                    transition: { duration: 0.36, times: [0, 0.55, 1], ease: DRAWER },
                  }
            }
          >
            <div className="pill-noise" aria-hidden />
            <AnimatePresence initial={true} mode="popLayout" propagate>
              {content === 'wave' && (
                <motion.div key="wave" className="absolute inset-0 flex items-center justify-center" {...swap}>
                  <Wave active reduced={reduced} />
                </motion.div>
              )}
              {content === 'loader' && (
                <motion.div
                  key="loader"
                  className="absolute inset-y-0 left-0 flex items-center"
                  style={{ paddingLeft: state.phase === 'formatting' ? PAD_L : (LOADER_W - LOADER_SPAN) / 2 }}
                  {...swap}
                  layout="position"
                  transition={{ layout: { type: 'spring', duration: 0.4, bounce: 0.12 } }}
                >
                  <Loader reduced={reduced} delay={contentDelay} />
                  <AnimatePresence initial={false}>
                    {label && (
                      <motion.span
                        key={label}
                        className="pill-label pill-gradient-text whitespace-nowrap"
                        style={{ marginLeft: TEXT_GAP }}
                        initial={reduced ? { opacity: 0 } : { opacity: 0, x: -6, filter: 'blur(3px)' }}
                        animate={{ opacity: 1, x: 0, filter: 'blur(0px)', transition: { duration: 0.26, delay: 0.1, ease: EASE_OUT } }}
                        exit={{ opacity: 0, transition: { duration: 0.1 } }}
                      >
                        {label}
                      </motion.span>
                    )}
                  </AnimatePresence>
                </motion.div>
              )}
              {content === 'message' && (
                <motion.div key="msg" className="absolute inset-0 flex items-center justify-center px-7" {...swap}>
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

/** Dot, bar, dot: they pop in one after another, then the bar's height travels. */
function Loader({ reduced, delay }: { reduced: boolean; delay: number }) {
  return (
    <div className="flex items-center gap-[11px]" data-reduced={reduced || undefined}>
      {[0, 1, 2].map((i) => (
        <motion.span
          key={i}
          className="pill-blob loader-blob"
          style={{ animationDelay: `${i * 200}ms` }}
          initial={reduced ? false : { scale: 0.3, opacity: 0 }}
          animate={{ scale: 1, opacity: 0.85, transition: { type: 'spring', duration: 0.42, bounce: 0.35, delay: delay + i * 0.05 } }}
        />
      ))}
    </div>
  );
}
