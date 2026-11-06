/** Two linked nodes: devices meeting through a relay. */
export function BrandMark({ size = 32 }: { size?: number }) {
  return (
    <span className="brand-mark" style={{ width: size, height: size }}>
      <svg width={size * 0.6} height={size * 0.6} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <circle cx="6" cy="12" r="3.2" stroke="currentColor" strokeWidth="2" />
        <circle cx="18" cy="6" r="2.6" fill="currentColor" />
        <circle cx="18" cy="18" r="2.6" fill="currentColor" />
        <path d="M9 10.6 15.6 7.2M9 13.4l6.6 3.4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      </svg>
    </span>
  );
}
