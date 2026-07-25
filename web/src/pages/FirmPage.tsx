import { useEffect, useLayoutEffect, useRef } from "react";
import { Link, useParams } from "react-router-dom";
import { createChart, type IChartApi, type UTCTimestamp } from "lightweight-charts";
import { AnalyticsLayout } from "./AnalyticsLayout";
import { useGameSocket } from "../hooks/useGameSocket";
import { topProducts } from "../lib/derive";
import { compact, signedCompact } from "../lib/format";
import { roleColor } from "../lib/roles";
import "./analytics.css";
import { PRODUCT_LABEL } from "../lib/catalog";

const CITY_LABEL: Record<string, string> = {
  istanbul: "İstanbul", ankara: "Ankara", izmir: "İzmir", bursa: "Bursa", konya: "Konya",
};

function PnlChart({ points }: { points: { tick: number; pnl: number }[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const c = createChart(el, {
      layout: { background: { color: "transparent" }, textColor: "rgba(150,150,160,0.6)", fontFamily: "IBM Plex Mono", fontSize: 10 },
      grid: { vertLines: { color: "rgba(55,57,66,0.35)" }, horzLines: { color: "rgba(55,57,66,0.35)" } },
      rightPriceScale: { borderColor: "rgba(55,57,66,0.5)" },
      timeScale: { borderColor: "rgba(55,57,66,0.5)", tickMarkFormatter: (t: UTCTimestamp) => `t${t}`, timeVisible: false, secondsVisible: false },
      crosshair: { vertLine: { color: "rgba(130,130,150,0.4)", width: 1 }, horzLine: { color: "rgba(130,130,150,0.4)", width: 1 } },
      width: el.clientWidth, height: el.clientHeight,
      handleScroll: false, handleScale: false,
    });
    const series = c.addBaselineSeries({
      baseValue: { type: "price", price: 0 },
      topLineColor: "rgba(110,190,140,0.9)", topFillColor1: "rgba(110,190,140,0.22)", topFillColor2: "rgba(110,190,140,0.02)",
      bottomLineColor: "rgba(200,110,95,0.9)", bottomFillColor1: "rgba(200,110,95,0.02)", bottomFillColor2: "rgba(200,110,95,0.22)",
      lineWidth: 2, priceLineVisible: false, lastValueVisible: true,
    });
    chart.current = c;
    if (points.length) {
      series.setData(points.map(p => ({ time: p.tick as UTCTimestamp, value: p.pnl })));
      c.timeScale().fitContent();
    }
    const ro = new ResizeObserver(() => c.resize(el.clientWidth, el.clientHeight));
    ro.observe(el);
    return () => { ro.disconnect(); c.remove(); chart.current = null; };
  }, []);

  useEffect(() => {
    if (!chart.current || !points.length) return;
    // update not re-create — lightweight-charts handles this
  }, [points]);

  return <div ref={ref} style={{ width: "100%", height: "100%" }} />;
}

export function FirmPage() {
  const { id } = useParams<{ id: string }>();
  const { snapshot, history, tradeStats } = useGameSocket();

  if (!snapshot || !id) {
    return (
      <AnalyticsLayout>
        <div className="det"><div className="det__empty">yükleniyor…</div></div>
      </AnalyticsLayout>
    );
  }

  const playerId = Number(id);
  const player = snapshot.leaderboard.find(p => p.id === playerId);
  if (!player) {
    return (
      <AnalyticsLayout>
        <div className="det"><div className="det__empty">firma bulunamadı</div></div>
      </AnalyticsLayout>
    );
  }

  const color = roleColor(player.npc_kind);
  const isSanayici = player.npc_kind === "Sanayici";
  const isTuccar = player.npc_kind === "Tüccar";
  const sign = player.pnl_lira >= 0 ? "pos" : "neg";

  const factories = snapshot.factories.filter(f => f.owner === playerId);
  const privateFarms = snapshot.private_farms.filter(f => f.owner === playerId);
  const caravans = snapshot.caravans.filter(c => c.owner === playerId);
  const myRels = snapshot.relations
    .filter(r => r.player_a === playerId || r.player_b === playerId)
    .sort((a, b) => b.trust_score - a.trust_score)
    .slice(0, 8);
  const pnlHistory = history[playerId] ?? [];
  const products = topProducts(tradeStats, playerId);

  const activeFabs = factories.filter(f => !f.idle).length;
  const totalVol = products.reduce((s, p) => s + p.buy_qty + p.sell_qty, 0);

  return (
    <AnalyticsLayout>
      <div className="det">
        {/* Başlık */}
        <div className="det__head">
          <div className="det__breadcrumb">
            <Link to="/analytics">Analitik</Link>
            <span>›</span>
            <span>{player.npc_kind === "Tüccar" ? "Lojistik" : "Sanayi"}</span>
            <span>›</span>
            <span>{player.name}</span>
          </div>
          <div className="det__title-row">
            <span className="det__badge" style={{ color, borderColor: color }}>
              {player.npc_kind?.toUpperCase().slice(0, 3)}
            </span>
            <h1 className="det__title">{player.name}</h1>
            <span className="det__subtitle">#{snapshot.leaderboard.indexOf(player) + 1} sıra · t{snapshot.tick}</span>
          </div>
        </div>

        {/* KPI bar */}
        <div className="det__kpi-bar">
          <div className="det__kpi">
            <span className="det__kpi-l">PnL</span>
            <span className={`det__kpi-v num det__kpi-v--${sign}`}>{signedCompact(player.pnl_lira)}₺</span>
          </div>
          <div className="det__kpi">
            <span className="det__kpi-l">NAKİT</span>
            <span className="det__kpi-v num">{compact(player.cash_lira)}₺</span>
          </div>
          {isSanayici && (
            <div className="det__kpi">
              <span className="det__kpi-l">FABRİKA</span>
              <span className="det__kpi-v det__kpi-v--accent">{activeFabs}/{factories.length}</span>
            </div>
          )}
          {isSanayici && privateFarms.length > 0 && (
            <div className="det__kpi">
              <span className="det__kpi-l">TARLA</span>
              <span className="det__kpi-v det__kpi-v--accent">{privateFarms.length}</span>
            </div>
          )}
          {isTuccar && (
            <div className="det__kpi">
              <span className="det__kpi-l">KERVAN</span>
              <span className="det__kpi-v">{caravans.length}</span>
            </div>
          )}
          <div className="det__kpi">
            <span className="det__kpi-l">HACİM</span>
            <span className="det__kpi-v">{compact(totalVol)}</span>
          </div>
          <div className="det__kpi">
            <span className="det__kpi-l">İLİŞKİ</span>
            <span className="det__kpi-v">{myRels.length}</span>
          </div>
        </div>

        {/* PnL grafiği — tam genişlik */}
        <div className="det__chart">
          {pnlHistory.length > 1
            ? <PnlChart points={pnlHistory} />
            : <div className="det__empty" style={{ height: "100%" }}>PnL geçmişi birikiyor…</div>
          }
        </div>

        {/* İki kolon body */}
        <div className="det__body" style={{ flex: 1 }}>
          {/* SOL: varlıklar */}
          <div className="det__col">
            {/* Fabrikalar (Sanayici) */}
            {isSanayici && factories.length > 0 && (
              <div className="det__section">
                <div className="det__section-title">Fabrikalar</div>
                <div className="fab-grid">
                  {factories.map(f => (
                    <div key={f.id} className={`fab-card ${f.idle ? "fab-card--idle" : ""}`}>
                      <div className="fab-card__product">{PRODUCT_LABEL[f.product] ?? f.product}</div>
                      <div className="fab-card__city">{CITY_LABEL[f.city] ?? f.city}</div>
                      <div className="fab-card__status">{f.idle ? "ATIL" : `${f.pending_units} bk bekliyor`}</div>
                      <div className="fab-card__bar">
                        <div className="fab-card__bar-fill" style={{ width: f.idle ? "0%" : "100%" }} />
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Özel Tarlalar */}
            {isSanayici && privateFarms.length > 0 && (
              <div className="det__section">
                <div className="det__section-title">Özel Tarlalar <span style={{ color: "var(--gain-dim)", fontSize: "0.6rem" }}>münhasır arz</span></div>
                {privateFarms.map(f => (
                  <div key={f.id} className="farm-row">
                    <span className="farm-row__icon">🌾</span>
                    <span className="farm-row__prod">{PRODUCT_LABEL[f.product] ?? f.product}</span>
                    <span className="farm-row__city">{CITY_LABEL[f.city] ?? f.city}</span>
                    <span className="farm-row__tag">sadece bana</span>
                  </div>
                ))}
              </div>
            )}

            {/* Kervanlar (Tüccar) */}
            {isTuccar && (
              <div className="det__section">
                <div className="det__section-title">Kervanlar</div>
                {caravans.length === 0 && <div style={{ color: "var(--text-ghost)", fontSize: "var(--text-sm)" }}>yok</div>}
                {caravans.map(c => (
                  <div key={c.id} className="caravan-row">
                    <span className="caravan-row__id">#{c.id}</span>
                    <span>{c.idle
                      ? `demirli · ${CITY_LABEL[c.current_city ?? ""] ?? c.current_city ?? "—"}`
                      : `yolda · ${c.cargo_units} bk`}
                    </span>
                    <span className={`caravan-row__status ${c.idle ? "caravan-row__status--idle" : "caravan-row__status--moving"}`}>
                      {c.idle ? "BEKLE" : "HAREKET"}
                    </span>
                  </div>
                ))}
              </div>
            )}

            {/* Sezon İşlemleri */}
            {products.length > 0 && (
              <div className="det__section">
                <div className="det__section-title">Sezon İşlemleri</div>
                <table className="trade-table">
                  <thead>
                    <tr>
                      <th>Ürün</th>
                      <th className="num gain">Aldı</th>
                      <th className="num loss">Sattı</th>
                      <th className="num">Toplam</th>
                    </tr>
                  </thead>
                  <tbody>
                    {products.map(p => (
                      <tr key={p.product}>
                        <td>{PRODUCT_LABEL[p.product] ?? p.product}</td>
                        <td className="num gain">{p.buy_qty > 0 ? compact(p.buy_qty) : "—"}</td>
                        <td className="num loss">{p.sell_qty > 0 ? compact(p.sell_qty) : "—"}</td>
                        <td className="num">{compact(p.buy_qty + p.sell_qty)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>

          {/* SAĞ: ilişkiler */}
          <div className="det__col">
            <div className="det__section" style={{ flex: 1 }}>
              <div className="det__section-title">Güven İlişkileri</div>
              {myRels.length === 0
                ? <div style={{ color: "var(--text-ghost)", fontSize: "var(--text-sm)" }}>henüz ilişki yok</div>
                : (
                  <div className="rel-list">
                    {myRels.map(r => {
                      const otherId = r.player_a === playerId ? r.player_b : r.player_a;
                      const other = snapshot.leaderboard.find(p => p.id === otherId);
                      const pct = Math.round(r.trust_score * 100);
                      const barColor = pct > 60 ? "var(--gain-dim)" : pct > 30 ? "var(--accent-dim)" : "var(--line-strong)";
                      return (
                        <div key={`${r.player_a}-${r.player_b}`} className="rel-item">
                          <div className="rel-top">
                            <span className="rel-name">{other?.name ?? `#${otherId}`}</span>
                            <span className="rel-kind">{other?.npc_kind ?? "?"}</span>
                            <span className="rel-count">{r.trade_count} işlem · {compact(r.total_units)} birim</span>
                          </div>
                          <div className="rel-bar-track">
                            <div className="rel-bar-fill" style={{ width: `${pct}%`, background: barColor }} />
                          </div>
                          <div style={{ textAlign: "right", fontSize: "0.6rem", color: "var(--text-ghost)", marginTop: "0.15rem" }}>
                            %{pct} güven
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )
              }
            </div>
          </div>
        </div>
      </div>
    </AnalyticsLayout>
  );
}
