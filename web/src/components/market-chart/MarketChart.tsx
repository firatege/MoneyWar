import type { PriceCell, PricePoint } from "../../types";
import { lira2 } from "../../lib/format";
import { TrendChart } from "../trend-chart/TrendChart";
import "./market-chart.css";

interface Props {
  cell: PriceCell | null;
  points: PricePoint[];
}

export function MarketChart({ cell, points }: Props) {
  const pct =
    cell && cell.last_lira != null && cell.baseline_lira > 0
      ? ((cell.last_lira - cell.baseline_lira) / cell.baseline_lira) * 100
      : null;
  const dir =
    pct == null ? "flat" : pct > 0.05 ? "up" : pct < -0.05 ? "down" : "flat";

  const trendPoints = points.map((p) => ({ tick: p.tick, value: p.lira }));

  return (
    <div className="mc">
      <div className="mc__head">
        <div className="mc__id">
          <span className="mc__product">{cell?.product_label ?? "—"}</span>
          <span className="mc__city">{cell?.city_label ?? ""}</span>
          {cell && (
            <span className="mc__raw-badge">{cell.is_raw ? "HAM" : "MAMUL"}</span>
          )}
        </div>
        <div className="mc__stats">
          <span className="mc__stat">
            <span className="mc__stat-l">SON</span>
            <span className="mc__stat-v num">{lira2(cell?.last_lira ?? null)}</span>
          </span>
          <span className="mc__stat">
            <span className="mc__stat-l">ORT5</span>
            <span className="mc__stat-v num">{lira2(cell?.avg5_lira ?? null)}</span>
          </span>
          <span className="mc__stat">
            <span className="mc__stat-l">BAZE</span>
            <span className="mc__stat-v num">{lira2(cell?.baseline_lira ?? null)}</span>
          </span>
          {pct != null && (
            <span className={`mc__delta mc__delta--${dir}`}>
              {pct > 0 ? "+" : ""}
              {pct.toFixed(1)}%
            </span>
          )}
        </div>
      </div>
      <TrendChart
        points={trendPoints}
        baseline={cell?.baseline_lira ?? 0}
        emptyText={cell ? "veri bekleniyor…" : "ürün seçin"}
      />
      <div className="mc__foot">
        <span>
          AL: {cell?.buy_qty ?? 0} · SAT: {cell?.sell_qty ?? 0}
        </span>
        <span>
          BID: {lira2(cell?.bid_lira ?? null)} · ASK: {lira2(cell?.ask_lira ?? null)}
        </span>
      </div>
    </div>
  );
}
