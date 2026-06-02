import type { FeedItem, PriceCell, PricePoint } from "../../types";
import { lira2, tickLabel } from "../../lib/format";
import { TrendChart } from "../trend-chart/TrendChart";
import "./market-chart.css";

interface Props {
  cell: PriceCell | null;
  points: PricePoint[];
  feed: FeedItem[];
}

export function MarketChart({ cell, points, feed }: Props) {
  const pct =
    cell && cell.last_lira != null && cell.baseline_lira > 0
      ? ((cell.last_lira - cell.baseline_lira) / cell.baseline_lira) * 100
      : null;
  const dir =
    pct == null ? "flat" : pct > 0.05 ? "up" : pct < -0.05 ? "down" : "flat";

  const trendPoints = points.map((p) => ({ tick: p.tick, value: p.lira }));

  // Bu bucket'ta yapılan işlemler (eşleşmeler).
  const trades = cell
    ? feed.filter(
        (e) => e.kind === "match" && e.city === cell.city && e.product === cell.product,
      )
    : [];
  const volume = trades.reduce((s, t) => s + (t.qty ?? 0), 0);
  const spread =
    cell?.bid_lira != null && cell?.ask_lira != null
      ? cell.ask_lira - cell.bid_lira
      : null;

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
          <Stat label="SON" value={lira2(cell?.last_lira ?? null)} />
          <Stat label="ORT5" value={lira2(cell?.avg5_lira ?? null)} />
          <Stat label="BAZE" value={lira2(cell?.baseline_lira ?? null)} />
          {pct != null && (
            <span className={`mc__delta mc__delta--${dir}`}>
              {pct > 0 ? "+" : ""}
              {pct.toFixed(1)}%
            </span>
          )}
        </div>
      </div>

      <div className="mc__body">
        <div className="mc__chart-side">
          <TrendChart
            points={trendPoints}
            baseline={cell?.baseline_lira ?? 0}
            emptyText={cell ? "veri bekleniyor…" : "ürün seçin"}
          />
        </div>

        <aside className="mc__trades">
          <div className="mc__trades-head">
            <span className="mc__trades-title">BU BUCKETTA İŞLEMLER</span>
            <span className="mc__trades-meta">
              {trades.length} işlem · {volume} bk
            </span>
          </div>
          <div className="mc__trades-list">
            {trades.length === 0 && (
              <div className="mc__trades-empty">
                {cell ? "henüz eşleşme yok" : "ürün seçin"}
              </div>
            )}
            {trades.map((t) => (
              <Trade key={t.key} trade={t} />
            ))}
          </div>
        </aside>
      </div>

      <div className="mc__foot">
        <span className="mc__foot-stat">
          <i className="mc__dot mc__dot--bid" />
          BID {lira2(cell?.bid_lira ?? null)} · {cell?.buy_qty ?? 0} bk
        </span>
        <span className="mc__foot-stat">
          SPREAD {spread != null ? spread.toFixed(2) : "—"}
        </span>
        <span className="mc__foot-stat">
          ASK {lira2(cell?.ask_lira ?? null)} · {cell?.sell_qty ?? 0} bk
          <i className="mc__dot mc__dot--ask" />
        </span>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <span className="mc__stat">
      <span className="mc__stat-l">{label}</span>
      <span className="mc__stat-v num">{value}</span>
    </span>
  );
}

/** Tek işlem satırı — taraflar (summary'den) + miktar × fiyat. */
function Trade({ trade }: { trade: FeedItem }) {
  // summary: "Alici-112 → Sanayici-106 · 1× Zeytinyağı @ 68.9₺ (Konya)"
  const parties = trade.summary.split(" · ")[0] ?? trade.summary;
  return (
    <div className="mc__trade">
      <span className="mc__trade-tick num">{tickLabel(trade.tick)}</span>
      <span className="mc__trade-parties">{parties}</span>
      <span className="mc__trade-qp num">
        {trade.qty ?? "—"}× @ {trade.price_lira != null ? lira2(trade.price_lira) : "—"}
      </span>
    </div>
  );
}
