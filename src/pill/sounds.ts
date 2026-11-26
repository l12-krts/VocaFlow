/**
 * Synthesised cues: no audio files, no decode latency, and they sit well
 * under music. Two soft sine partials with a fast attack and an exponential
 * tail; start rises, stop falls, cancel is a single low note.
 */
let ctx: AudioContext | null = null;

function tone(freqs: [number, number], at: number, dur: number, gain: number) {
  const c = ctx!;
  const g = c.createGain();
  g.gain.setValueAtTime(0, at);
  g.gain.linearRampToValueAtTime(gain, at + 0.008);
  g.gain.exponentialRampToValueAtTime(0.0001, at + dur);
  g.connect(c.destination);
  for (const [i, f] of freqs.entries()) {
    const o = c.createOscillator();
    o.type = 'sine';
    o.frequency.setValueAtTime(f, at);
    const pg = c.createGain();
    pg.gain.value = i === 0 ? 1 : 0.18; // faint octave for "glass"
    o.connect(pg).connect(g);
    o.start(at);
    o.stop(at + dur + 0.02);
  }
}

export function playCue(kind: 'start' | 'stop' | 'cancel') {
  try {
    ctx ??= new AudioContext();
    if (ctx.state === 'suspended') void ctx.resume();
    const t = ctx.currentTime + 0.005;
    const v = 0.055;
    if (kind === 'start') {
      tone([740, 1480], t, 0.11, v);
      tone([988, 1976], t + 0.07, 0.16, v);
    } else if (kind === 'stop') {
      tone([988, 1976], t, 0.1, v);
      tone([740, 1480], t + 0.06, 0.15, v);
    } else {
      tone([440, 880], t, 0.16, v * 0.9);
    }
  } catch {
    /* audio is a nicety; never fail dictation over it */
  }
}
