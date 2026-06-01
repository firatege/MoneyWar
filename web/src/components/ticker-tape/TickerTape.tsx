import type { PriceCell } from "../../types";
import { lira2 } from "../../lib/format";
import "./ticker-tape.css";

interface Props {
  prices: PriceCell[];
}

function delta(cell: PriceCell): number | null {
  if (cell.last_lira == null || cell.baseline_lira <= 0) return null;
  return ((cell.last_lira - cell.baseline_lira) / cell.baseline_lira) * 100;
}

function Cell({ cell }: { cell: PriceCell }) {
  const d = delta(cell);
  const dir = d == null ? "flat" : d > 0.05 ? "up" : d < -0.05 ? "down" : "flat";
  const arrow = dir === "up" ? "+" : dir === "down" ? "−" : "·";
  return (
    <span className="tick-cell">
      <span className="tick-cell__name">
        {cell.product_label}
        <span className="tick-cell__city">{cell.city_label}</span>
      </span>
      <span className="tick-cell__price num">{lira2(cell.last_lira)}</span>
      <span className={`tick-cell__delta tick-cell__delta--${dir}`}>
        {arrow}
        {d == null ? "—" : Math.abs(d).toFixed(1)}
      </span>
    </span>
  );
}

export function TickerTape({ prices }: Props) {
  if (prices.length === 0) {
    return <div className="ticker ticker--empty">akış bekleniyor…</div>;
  }
  // Kesintisiz döngü için içerik iki kez basılır.
  const run = (
    <div className="ticker__run">
      {prices.map((c) => (
        <Cell key={`${c.city}-${c.product}`} cell={c} />
      ))}
    </div>
  );
  return (
    <div className="ticker" aria-label="fiyat akışı">
      <div className="ticker__marquee">
        {run}
        {run}
      </div>
    </div>
  );
}
