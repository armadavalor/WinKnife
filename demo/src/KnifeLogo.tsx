// The app's knife logo (src/lib/components/KnifeLogo.svelte), drawn as the
// same vector. `open` is 0 for the folded blade and 1 for the open one; the
// blade turns about the pivot (645, 422), as in the app's fold animation.
const FOLDED_DEG = -88;

export const KnifeLogo = ({size, open}: {size: number; open: number}) => (
  <svg viewBox="0 0 1024 1024" width={size} height={size}>
    <circle cx="512" cy="512" r="512" fill="#0090e3" />
    {/* Blade first, so it folds away behind the handle. */}
    <g transform={`rotate(${FOLDED_DEG * (1 - open)} 645 422)`}>
      <g transform="translate(645 422) rotate(-139.5)">
        <path
          d="M-30 -62 L250 -62 C350 -62 420 0 400 62 L-30 62 Z"
          fill="#bdbab9"
          stroke="#0b4d73"
          strokeWidth="26"
          strokeLinejoin="round"
        />
        <path
          d="M0 -42 L248 -42 C330 -42 385 -6 380 40"
          fill="none"
          stroke="#ffffff"
          strokeWidth="18"
          strokeLinecap="round"
        />
      </g>
    </g>
    <g transform="translate(767 431) rotate(39.7)">
      <path
        d="M-93 0 H93 V455 a70 70 0 0 1 -70 70 H-23 a70 70 0 0 1 -70 -70 Z"
        fill="#0c6499"
        stroke="#0b4d73"
        strokeWidth="26"
        strokeLinejoin="round"
      />
      <g fill="#0b4d73" transform="translate(-36 380)">
        <rect width="34" height="34" />
        <rect x="40" width="34" height="34" />
        <rect y="40" width="34" height="34" />
        <rect x="40" y="40" width="34" height="34" />
      </g>
    </g>
  </svg>
);
