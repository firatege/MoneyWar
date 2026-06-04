import { useLayoutEffect, useRef, useEffect } from "react";
import { Link } from "react-router-dom";
import { createChart, type IChartApi, type ISeriesApi, type UTCTimestamp } from "lightweight-charts";
import { AnalyticsLayout } from "./AnalyticsLayout";
import { useGameSocket } from "../hooks/useGameSocket";
import { signedCompact } from "../lib/format";
import "./analytics.css";

function MultiLineChart({ series }: {
  series: { name: string; color: string; points: { tick: number; value: number }[] }[]
}) {
  const ref = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRefs = useRef<ISeriesApi<"Line">[]>([]);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const c = createChart(el, {
      layout: { background: { color: "transparent" }, textColor: "rgba(150,150,160,0.6)", fontFamily: "IBM Plex Mono", fontSize: 10 },
      grid: { vertLines: { color: "rgba(55,57,66,0.3)" }, horzLines: { color: "rgba(55,57,66,0.3)" } },
      rightPriceScale: { borderColor: "rgba(55,57,66,0.4)" },
      timeScale: { borderColor: "rgba(55,57,66,0.4)", tickMarkFormatter: (t: UTCTimestamp) => `t${t}`, timeVisible: false, secondsVisible: false },
      width: el.clientWidth, height: el.clientHeight,
      handleScroll: false, handleScale: false,
    });
    chartRef.current = c;
    seriesRefs.current = series.map(s => {
      const ls = c.addLineSeries({ color: s.color, lineWidth: 2, priceLineVisible: false, lastValueVisible: true, title: s.name });
      if (s.points.length) ls.setData(s.points.map(p => ({ time: p.tick as UTCTimestamp, value: p.value })));
      return ls;
    });
    if (series.some(s => s.points.length)) c.timeScale().fitContent();
    const ro = new ResizeObserver(() => c.resize(el.clientWidth, el.clientHeight));
    ro.observe(el);
    return () => { ro.disconnect(); c.remove(); };
  }, []);

  useEffect(() => {
    if (!chartRef.current) return;
    seriesRefs.current.forEach((s, i) => {
      if (series[i]?.points.length) s.setData(series[i].points.map(p => ({ time: p.tick as UTCTimestamp, value: p.value })));
    });
    if (series.some(s => s.points.length)) chartRef.current.timeScale().fitContent();
  }, [series]);

  return <div ref={ref} style={{ width: "100%", height: "100%" }} />;
}

export function MarketPage() {
  const { snapshot, market } = useGameSocket();

  if (!snapshot) {
    return (
      <AnalyticsLayout>
        <div className="det"><div className="det__empty">yükleniyor…</div></div>
      </AnalyticsLayout>
    );
  }

  // Fiyat indeksleri — mamul ürünler için son fiyat/baseline
  const indexSeries = [
    {
      name: "Kumaş",
      color: "var(--role-sanayici)",
      points: market.map(m => ({ tick: m.tick, value: m.finIndex })),
    },
    {
      name: "Ham",
      color: "var(--role-ciftci)",
      points: market.map(m => ({ tick: m.tick, value: m.rawIndex })),
    },
    {
      name: "Genel",
      color: "var(--accent)",
      points: market.map(m => ({ tick: m.tick, value: m.index })),
    },
  ];

  const volumeSeries = [
    {
      name: "Hacim",
      color: "rgba(110,190,140,0.7)",
      points: market.map(m => ({ tick: m.tick, value: m.volume })),
    },
  ];

  // Rol bazlı PnL
  const roles = ["Tüccar", "Sanayici", "Alıcı", "Spekülatör", "Çiftçi"];
  const roleColors: Record<string, string> = {
    "Tüccar": "var(--role-tuccar)", "Sanayici": "var(--role-sanayici)",
    "Alıcı": "var(--role-alici)", "Spekülatör": "var(--role-spekulator)", "Çiftçi": "var(--role-ciftci)",
  };
  const rolePnl = roles.map(role => {
    const players = snapshot.leaderboard.filter(p => p.npc_kind === role);
    const total = players.reduce((s, p) => s + p.pnl_lira, 0);
    return { role, total, count: players.length };
  }).filter(r => r.count > 0).sort((a, b) => b.total - a.total);

  const maxPnl = Math.max(...rolePnl.map(r => Math.abs(r.total)), 1);

  // İlişki ağı — en güçlü bağlar (firma bazlı)
  const topRels = [...snapshot.relations]
    .sort((a, b) => b.trust_score - a.trust_score)
    .slice(0, 20);

  const getName = (id: number) =>
    snapshot.leaderboard.find(p => p.id === id)?.name ?? `#${id}`;

  // Firma başına en güçlü ilişkileri grupla
  const firmRels: Record<string, { partner: string; trust: number; count: number }[]> = {};
  for (const rel of topRels) {
    const nameA = getName(rel.player_a);
    const nameB = getName(rel.player_b);
    if (!firmRels[nameA]) firmRels[nameA] = [];
    if (!firmRels[nameB]) firmRels[nameB] = [];
    if (firmRels[nameA].length < 3) firmRels[nameA].push({ partner: nameB, trust: rel.trust_score, count: rel.trade_count });
    if (firmRels[nameB].length < 3) firmRels[nameB].push({ partner: nameA, trust: rel.trust_score, count: rel.trade_count });
  }

  const firms = Object.entries(firmRels).slice(0, 8);

  return (
    <AnalyticsLayout>
      <div className="det">
        <div className="det__head">
          <div className="det__breadcrumb">
            <Link to="/analytics">Analitik</Link>
            <span>›</span>
            <span>Piyasa Genel</span>
          </div>
          <div className="det__title-row">
            <h1 className="det__title">Piyasa Analitik</h1>
            <span className="det__subtitle">t{snapshot.tick} · sezon {snapshot.season}</span>
          </div>
        </div>

        <div className="det__kpi-bar">
          {rolePnl.map(r => (
            <div key={r.role} className="det__kpi">
              <span className="det__kpi-l">{r.role.toUpperCase()}</span>
              <span className={`det__kpi-v num ${r.total >= 0 ? "det__kpi-v--pos" : "det__kpi-v--neg"}`}>
                {signedCompact(r.total)}₺
              </span>
            </div>
          ))}
        </div>

        <div className="mkt__grid" style={{ flex: 1 }}>
          {/* Fiyat indeksleri */}
          <div className="mkt__panel mkt__panel--full">
            <div className="mkt__panel-head">
              FİYAT ENDEKSİ <span style={{ color: "var(--text-ghost)", fontWeight: 400 }}>· 100 = baz fiyat</span>
              <span style={{ float: "right", display: "flex", gap: "1rem" }}>
                {indexSeries.map(s => (
                  <span key={s.name} style={{ fontSize: "0.65rem", color: s.color }}>● {s.name}</span>
                ))}
              </span>
            </div>
            <div className="mkt__chart" style={{ height: "180px" }}>
              <MultiLineChart series={indexSeries} />
            </div>
          </div>

          {/* İşlem hacmi */}
          <div className="mkt__panel">
            <div className="mkt__panel-head">İŞLEM HACMİ</div>
            <div className="mkt__chart">
              <MultiLineChart series={volumeSeries} />
            </div>
          </div>

          {/* Rol PnL karşılaştırması */}
          <div className="mkt__panel">
            <div className="mkt__panel-head">ROL BAZLI PnL</div>
            <div style={{ padding: "0.85rem 1.25rem", display: "flex", flexDirection: "column", gap: "0.6rem" }}>
              {rolePnl.map(r => (
                <div key={r.role} style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
                  <div style={{ display: "flex", justifyContent: "space-between", fontSize: "var(--text-xs)" }}>
                    <span style={{ color: roleColors[r.role] ?? "var(--text-dim)" }}>{r.role}</span>
                    <span style={{ color: r.total >= 0 ? "var(--gain-dim)" : "var(--loss-dim)" }}>
                      {signedCompact(r.total)}₺
                    </span>
                  </div>
                  <div style={{ height: "4px", background: "var(--surface-hi)", borderRadius: "2px" }}>
                    <div style={{
                      width: `${(Math.abs(r.total) / maxPnl) * 100}%`,
                      height: "100%",
                      background: r.total >= 0 ? "var(--gain-dim)" : "var(--loss-dim)",
                      borderRadius: "2px",
                      transition: "width 0.5s",
                    }} />
                  </div>
                </div>
              ))}
            </div>
          </div>

          {/* İlişki ağı */}
          <div className="mkt__panel mkt__panel--full">
            <div className="mkt__panel-head">İLİŞKİ AĞI <span style={{ color: "var(--text-ghost)", fontWeight: 400 }}>· en güçlü bağlar</span></div>
            <div className="rel-matrix">
              {firms.map(([firmName, rels]) => (
                <div key={firmName} className="rel-matrix-row">
                  <div className="rel-matrix-name">{firmName}</div>
                  <div className="rel-matrix-bars">
                    {rels.map((r, i) => (
                      <div key={i} className="rel-matrix-bar-row">
                        <div className="rel-matrix-label">{r.partner}</div>
                        <div className="rel-matrix-track">
                          <div className="rel-matrix-fill" style={{
                            width: `${r.trust * 100}%`,
                            background: r.trust > 0.6 ? "var(--gain-dim)" : r.trust > 0.3 ? "var(--accent-dim)" : "var(--line-strong)",
                          }} />
                        </div>
                        <div className="rel-matrix-score">%{Math.round(r.trust * 100)}</div>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
              {firms.length === 0 && (
                <div style={{ color: "var(--text-ghost)", fontSize: "var(--text-sm)" }}>ilişki verisi birikiyor…</div>
              )}
            </div>
          </div>
        </div>
      </div>
    </AnalyticsLayout>
  );
}
