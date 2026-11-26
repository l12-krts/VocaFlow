import { alloStyle } from 'allo-gr/react';

/** The VocaFlow mark (logo_2.svg), drawn in currentColor. */
export function Logo({ size = 20, className }: { size?: number; className?: string }) {
  return (
    <svg
      width={(size * 150) / 120}
      height={size}
      viewBox="690 -120 150 120"
      className={className}
      fill="currentColor"
      aria-hidden
    >
      <path d="M 780 -90 L 810 -90 L 840 -90 L 840 -90.00003000000001 A 29.999969999999994 29.999969999999994 0 0 0 810.00003 -120 L 810 -120 L 780 -120 L 780 -90 Z" />
      <path d="M 780 -120 L 750 -120 L 750 -90 L 750 -60 L 750 -30 L 750 0 L 780 0 Z" />
      <path d="M 840 -60 L 810 -60 L 810 -30 L 824.9384373865579 -30 A 15.061562613442112 15.061562613442112 0 0 0 840 -45.06156261344211 L 840 -60 Z" />
      <path d="M 720 -120 L 720 -90 L 720 -89.99996999999999 A 29.999969999999994 29.999969999999994 0 0 0 749.99997 -60 L 750 -60 L 750 -90 L 750 -90.00003000000001 A 29.999969999999994 29.999969999999994 0 0 0 720.00003 -120 L 720 -120 Z" />
      <path d="M 719.99997 -120 L 720 -120 L 720 -90 L 720 -89.99996999999999 A 29.999969999999994 29.999969999999994 0 0 1 690.00003 -60 L 690 -60 L 690 -90 L 690 -90.00003000000001 A 29.999969999999994 29.999969999999994 0 0 1 719.99997 -120 Z" />
      <path d="M 719.99997 0 L 720 0 L 720 -30 L 720 -30.000030000000002 A 29.99996999999999 29.99996999999999 0 0 0 690.00003 -60 L 690 -60 L 690 -30 L 690 -29.999969999999998 A 29.99996999999999 29.99996999999999 0 0 0 719.99997 0 Z" />
      <path d="M 720 0 L 720 -30 L 720 -30.000030000000002 A 29.99996999999999 29.99996999999999 0 0 1 749.99997 -60 L 750 -60 L 750 -30 L 750 -29.999969999999998 A 29.99996999999999 29.99996999999999 0 0 1 720.00003 0 L 720 0 Z" />
    </svg>
  );
}

/**
 * Logo + "VocaFlow" set in Allo. Tracking is pulled in hard (-0.045em) so
 * the word reads as one shape next to the mark; optical size follows the
 * font size so small renders stay sturdy.
 */
export function Wordmark({ size = 20, className }: { size?: number; className?: string }) {
  return (
    <span className={`inline-flex items-center ${className ?? ''}`} style={{ gap: size * 0.38 }} aria-label="VocaFlow">
      <Logo size={size * 0.86} />
      <span
        aria-hidden
        style={{
          ...alloStyle({ weight: 720, roundness: 50 }),
          fontSize: size,
          lineHeight: 1,
          letterSpacing: '-0.045em',
          transform: 'translateY(-0.02em)',
        }}
      >
        VocaFlow
      </span>
    </span>
  );
}
