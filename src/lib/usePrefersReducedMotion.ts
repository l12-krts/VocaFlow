import { useEffect, useState } from 'react';

const q = typeof window !== 'undefined' ? window.matchMedia('(prefers-reduced-motion: reduce)') : null;

export function usePrefersReducedMotion() {
  const [reduced, setReduced] = useState(() => !!q?.matches);
  useEffect(() => {
    const f = () => setReduced(!!q?.matches);
    q?.addEventListener('change', f);
    return () => q?.removeEventListener('change', f);
  }, []);
  return reduced;
}
