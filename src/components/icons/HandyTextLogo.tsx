import React from "react";

/**
 * Kōrero wordmark — minimal edition (2026-05-15).
 *
 * Design philosophy: let the type carry the brand. The macron over ō is
 * already a distinctive identifier; no other speech app has it. Strip
 * everything else.
 *
 *   - Pure white wordmark, weight 700, tight tracking
 *   - One small cyan dot to the right of the word — a "live cursor" mark
 *     that hints at the app's job (dictation → text appearing)
 *   - Subtle 1px shadow for legibility on navy backgrounds (NOT a glow)
 *   - No aurora gradient, no glow filter, no underline, no waveform
 *
 * Filename kept as HandyTextLogo for import compatibility.
 */
const KoreroTextLogo = ({
  width,
  height,
  className,
}: {
  width?: number;
  height?: number;
  className?: string;
}) => {
  const w = width ?? 480;
  const h = height ?? Math.round(w * 0.32);
  return (
    <svg
      width={w}
      height={h}
      viewBox="0 0 480 156"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-label="Kōrero"
      role="img"
    >
      <defs>
        <filter id="korero-legibility" x="-5%" y="-5%" width="110%" height="120%">
          <feDropShadow dx="0" dy="1" stdDeviation="0.5" floodColor="#000000" floodOpacity="0.40" />
        </filter>
      </defs>

      {/* Wordmark — pure white, calm weight, tight tracking */}
      <text
        x="50%"
        y="56%"
        textAnchor="middle"
        dominantBaseline="middle"
        fontFamily="'Aptos', 'Aptos Display', 'Segoe UI Variable Text', ui-sans-serif, system-ui, -apple-system, 'Segoe UI', 'SF Pro Display', 'Inter', sans-serif"
        fontWeight={700}
        fontSize="92"
        letterSpacing="-2"
        fill="#FFFFFF"
        filter="url(#korero-legibility)"
      >
        Kōrero
      </text>

      {/* Cyan "live cursor" accent — single dot to the right of the wordmark */}
      <circle cx="408" cy="86" r="4.5" fill="#5DD8FF" />
    </svg>
  );
};

export default KoreroTextLogo;
export { KoreroTextLogo };
