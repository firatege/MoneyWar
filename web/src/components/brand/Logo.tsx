interface Props {
  size?: number;
  className?: string;
}

/**
 * MoneyWar logosu — sade candlestick (mum) kümesi. Borsa/işlem temsili,
 * süssüz. Renkler tema değişkenlerinden gelir.
 */
export function Logo({ size = 24, className }: Props) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      className={className}
      aria-hidden="true"
    >
      {/* çerçeve */}
      <rect
        x="0.75"
        y="0.75"
        width="22.5"
        height="22.5"
        rx="2"
        stroke="var(--line-strong)"
        strokeWidth="1"
      />
      {/* mum 1 — yükseliş */}
      <line x1="6" y1="5" x2="6" y2="19" stroke="var(--gain)" strokeWidth="1" />
      <rect x="4.5" y="9" width="3" height="7" fill="var(--gain)" />
      {/* mum 2 — düşüş */}
      <line x1="12" y1="4" x2="12" y2="20" stroke="var(--loss)" strokeWidth="1" />
      <rect x="10.5" y="7" width="3" height="6" fill="var(--loss)" />
      {/* mum 3 — aksan */}
      <line x1="18" y1="6" x2="18" y2="18" stroke="var(--accent)" strokeWidth="1" />
      <rect x="16.5" y="10" width="3" height="5" fill="var(--accent)" />
    </svg>
  );
}
