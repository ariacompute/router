import styles from './BrandMark.module.css';

// The mark is the Aria Compute favicon: a rounded-square badge carrying a
// stylised "A" — a diagonal stroke, a ring and a vertical stem. It takes its
// two inks from --brand-badge-bg / --brand-badge-fg (defined per theme in
// index.css), so the badge inverts between light and dark app themes and always
// pops off the surrounding surface. Mirrors agenthub's BrandMark.
type Props = { size?: number };

export default function BrandMark({ size = 30 }: Props) {
  return (
    <svg
      className={styles.glyph}
      width={size}
      height={size}
      viewBox="1 1 62 62"
      role="img"
      aria-label="Aria Compute"
    >
      <rect x="1" y="1" width="62" height="62" rx="18" className={styles.bg} />
      <g
        className={styles.fg}
        fill="none"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M12 53 23 11" strokeWidth={8} />
        <circle cx="41" cy="37" r="11" strokeWidth={6} />
        <path d="M51 54V27" strokeWidth={6} />
      </g>
    </svg>
  );
}
