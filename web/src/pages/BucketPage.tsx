import { useLayoutEffect, useRef, useEffect } from "react";
import { Link, useParams } from "react-router-dom";
import { createChart, type IChartApi, type ISeriesApi, type UTCTimestamp } from "lightweight-charts";
import { AnalyticsLayout } from "./AnalyticsLayout";
import { useGameSocket } from "../hooks/useGameSocket";
import { compact, lira2 } from "../lib/format";
import "./analytics.css";

const PRODUCT_LABEL: Record<string, string> = {
  kumas: "Kumaş", un: "Un", zeytinyagi: "Zeytinyağı",
  pamuk: "Pamuk", bugday: "Buğday", zeytin: "Zeytin",
};
const CITY_LABEL: Record<string, string> = {
  istanbul: "İstanbul", ankara: "Ankara", izmir: "İzmir", bursa: "Bursa", konya: "Konya",
};

// Sabit renk paleti
const PALETTE = [
  "var(--role-sanayici)", "var(--role-tuccar)", "var(--role-ciftci)",
  "var(--role-spekulator)", "var(--accent)", "var(--gain-dim)", "var(--loss-dim)",
];

function PriceChart({ points }: { points: { tick: number; value: number }[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRef = useRef<ISeriesApi<"Area"> | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const c = createChart(el, {
      layout: { background: { color: "transparent" }, textColor: "rgba(150,150,160,0.6)", fontFamily: "IBM Plex Mono", fontSize: 10 },
      grid: { vertLines: { color: "rgba(55,57,66,0.3)" }, horzLines: { color: "rgba(55,57,66,0.3)" } },
      rightPriceScale: { borderColor: "rgba(55,57,66,0.4)" },
      timeScale: { borderColor: "rgba(55,57,66,0.4)", tickMarkFormatter: (t: UTCTimestamp) => `t${t}`, timeVisible: false, secondsVisible: false },
      crosshair: { vertLine: { color: "rgba(130,130,150,0.4)", width: 1 }, horzLine: { color: "rgba(130,130,150,0.4)", width: 1 } },
      width: el.clientWidth, height: el.clientHeight,
      handleScroll: true, handleScale: true,
    });
    const s = c.addAreaSeries({
      lineColor: "rgba(110,190,140,0.9)",
      topColor: "rgba(110,190,140,0.2)",
      bottomColor: "rgba(110,190,140,0.01)",
      lineWidth: 2, priceLineVisible: false, lastValueVisible: true,
    });
    chartRef.current = c;
    seriesRef.current = s;
    if (points.length) {
      s.setData(points.map(p => ({ time: p.tick as UTCTimestamp, value: p.value })));
      c.timeScale().fitContent();
    }
    const ro = new ResizeObserver(() => c.resize(el.clientWidth, el.clientHeight));
    ro.observe(el);
    return () => { ro.disconnect(); c.remove(); };
  }, []);

  useEffect(() => {
    if (!seriesRef.current || !chartRef.current || !points.length) return;
    seriesRef.current.setData(points.map(p => ({ time: p.tick as UTCTimestamp, value: p.value })));
    chartRef.current.timeScale().fitContent();
  }, [points]);

  return <div ref={ref} style={{ width: "100%", height: "100%" }} />;
}

function SimpleBar({ items }: { items: { name: string; value: number; color: string }[] }) {
  const max = Math.max(...items.map(i => i.value), 1);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "0.4rem" }}>
      {items.map((item, idx) => (
        <div key={idx} style={{ display: "flex", alignItems: "center", gap: "0.5rem", fontSize: "var(--text-xs)" }}>
          <div style={{ width: "80px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--text-dim)" }}>
            {item.name}
          </div>
          <div style={{ flex: 1, height: "6px", background: "var(--surface-hi)", borderRadius: "3px", overflow: "hidden" }}>
            <div style={{ width: `${(item.value / max) * 100}%`, height: "100%", background: item.color, borderRadius: "3px", transition: "width 0.5s" }} />
          </div>
          <div style={{ width: "50px", textAlign: "right", color: "var(--text-faint)", fontSize: "0.65rem" }}>
            {compact(item.value)}
          </div>
        </div>
      ))}
    </div>
  );
}

export function BucketPage() {
  const { city, product } = useParams<{ city: string; product: string }>();
  const { snapshot, bucketHistory } = useGameSocket();

  if (!snapshot || !city || !product) {
    return (
      <AnalyticsLayout>
        <div className="det"><div className="det__empty">yükleniyor…</div></div>
      </AnalyticsLayout>
    );
  }

  const cell = snapshot.prices.find(p => p.city === city && p.product === product);
  const histKey = `${city}/${product}`;
  const priceHistory = (bucketHistory[histKey] ?? []).map((v, i) => ({ tick: i + 1, value: v }));

  // Bu bucket'ta hangi firmalar işlem yapıyor — events'ten
  const recentTrades = snapshot.recent_events.filter(e =>
    e.kind === "match" && e.city === city && e.product === product
  );

  // Satıcı bazlı hacim
  const sellerVol: Record<number, number> = {};
  const buyerVol: Record<number, number> = {};
  for (const e of recentTrades) {
    if (e.seller_id != null) sellerVol[e.seller_id] = (sellerVol[e.seller_id] ?? 0) + (e.qty ?? 0);
    if (e.buyer_id != null) buyerVol[e.buyer_id] = (buyerVol[e.buyer_id] ?? 0) + (e.qty ?? 0);
  }

  const getName = (id: number) =>
    snapshot.leaderboard.find(p => p.id === id)?.name ?? `#${id}`;

  const sellers = Object.entries(sellerVol)
    .sort((a, b) => b[1] - a[1])
    .map(([id, vol], i) => ({ name: getName(Number(id)), value: vol, color: PALETTE[i % PALETTE.length] }));

  const buyers = Object.entries(buyerVol)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 5)
    .map(([id, vol], i) => ({ name: getName(Number(id)), value: vol, color: PALETTE[i % PALETTE.length] }));

  // Özel tarlalar bu bucket'ta
  const farmsHere = product && !["kumas","un","zeytinyagi"].includes(product)
    ? snapshot.private_farms.filter(f => f.city === city && f.product === product)
    : [];

  const pct = cell && cell.baseline_lira > 0 && cell.last_lira
    ? ((cell.last_lira - cell.baseline_lira) / cell.baseline_lira * 100) : 0;

  return (
    <AnalyticsLayout>
      <div className="det">
        <div className="det__head">
          <div className="det__breadcrumb">
            <Link to="/analytics">Analitik</Link>
            <span>›</span>
            <span>Bucketlar</span>
            <span>›</span>
            <span>{PRODUCT_LABEL[product] ?? product} · {CITY_LABEL[city] ?? city}</span>
          </div>
          <div className="det__title-row">
            <h1 className="det__title">{PRODUCT_LABEL[product] ?? product}</h1>
            <span style={{ color: "var(--text-faint)", fontSize: "var(--text-sm)" }}>{CITY_LABEL[city] ?? city}</span>
            <span className="det__subtitle" style={{ color: pct >= 0 ? "var(--gain)" : "var(--loss)" }}>
              {pct >= 0 ? "+" : ""}{pct.toFixed(1)}% baseline
            </span>
          </div>
        </div>

        {/* KPI */}
        <div className="det__kpi-bar">
          <div className="det__kpi">
            <span className="det__kpi-l">SON FİYAT</span>
            <span className="det__kpi-v num">{lira2(cell?.last_lira)}₺</span>
          </div>
          <div className="det__kpi">
            <span className="det__kpi-l">ORT5</span>
            <span className="det__kpi-v num">{lira2(cell?.avg5_lira)}₺</span>
          </div>
          <div className="det__kpi">
            <span className="det__kpi-l">BASELINE</span>
            <span className="det__kpi-v num">{lira2(cell?.baseline_lira)}₺</span>
          </div>
          <div className="det__kpi">
            <span className="det__kpi-l">BİD</span>
            <span className="det__kpi-v num det__kpi-v--pos">{lira2(cell?.bid_lira)}₺</span>
          </div>
          <div className="det__kpi">
            <span className="det__kpi-l">ASK</span>
            <span className="det__kpi-v num det__kpi-v--neg">{lira2(cell?.ask_lira)}₺</span>
          </div>
          <div className="det__kpi">
            <span className="det__kpi-l">BEKLEYEN</span>
            <span className="det__kpi-v num">{cell?.buy_qty ?? 0} al / {cell?.sell_qty ?? 0} sat</span>
          </div>
        </div>

        {/* Büyük fiyat grafiği */}
        <div className="det__chart det__chart--tall">
          {priceHistory.length > 1
            ? <PriceChart points={priceHistory} />
            : <div className="det__empty" style={{ height: "100%" }}>fiyat geçmişi birikiyor…</div>
          }
        </div>

        {/* Alıcı / Satıcı dağılımı */}
        <div className="bkt__split">
          <div className="bkt__half">
            <div className="det__section-title">SATICI PAYI (bu tickte)</div>
            {sellers.length > 0
              ? <SimpleBar items={sellers} />
              : <div style={{ color: "var(--text-ghost)", fontSize: "var(--text-sm)" }}>veri yok</div>
            }
          </div>
          <div className="bkt__half">
            <div className="det__section-title">ALICI PAYI</div>
            {buyers.length > 0
              ? <SimpleBar items={buyers} />
              : <div style={{ color: "var(--text-ghost)", fontSize: "var(--text-sm)" }}>veri yok</div>
            }
          </div>
        </div>

        {/* Özel tarlalar */}
        {farmsHere.length > 0 && (
          <div className="det__section">
            <div className="det__section-title">ÖZEL TARLALAR</div>
            {farmsHere.map(f => {
              const owner = snapshot.leaderboard.find(p => p.id === f.owner);
              return (
                <div key={f.id} className="farm-row">
                  <span className="farm-row__icon">🌾</span>
                  <span className="farm-row__prod">{owner?.name ?? `#${f.owner}`}</span>
                  <span className="farm-row__tag">münhasır arz</span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </AnalyticsLayout>
  );
}
