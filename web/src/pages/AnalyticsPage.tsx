import { useState } from "react";
import { Link } from "react-router-dom";

import "./analytics.css";
import { useGameSocket } from "../hooks/useGameSocket";
import { topProducts } from "../lib/derive";
import { compact, lira, signedCompact } from "../lib/format";
import { roleColor } from "../lib/roles";
import type { PlayerDto, Snapshot } from "../types";
import "./pages.css";

const PRODUCT_LABEL: Record<string, string> = {
  pamuk: "Pamuk", bugday: "Buğday", zeytin: "Zeytin",
  kumas: "Kumaş", un: "Un", zeytinyagi: "Zeytinyağı",
};
const CITY_LABEL: Record<string, string> = {
  istanbul: "İst", ankara: "Ank", izmir: "İzm", bursa: "Bur", konya: "Kon",
};

export function AnalyticsPage() {
  const { snapshot, tradeStats } = useGameSocket();
  const [selectedId, setSelectedId] = useState<number | null>(null);

  if (!snapshot) {
    return (
      <div className="ana">
        <header className="ana__head">
          <Link to="/" className="ana__back">← Dashboard</Link>
          <h1 className="ana__title">ANALİTİK</h1>
        </header>
        <div className="ana__empty">bağlantı bekleniyor…</div>
      </div>
    );
  }

  const players = snapshot.leaderboard;
  const tuccars = players.filter(p => p.npc_kind === "Tüccar");
  const sanayicis = players.filter(p => p.npc_kind === "Sanayici");
  const alicilar = players.filter(p => p.npc_kind === "Alıcı");
  const speklar = players.filter(p => p.npc_kind === "Spekülatör");
  const selected = selectedId != null ? players.find(p => p.id === selectedId) ?? null : null;

  return (
    <div className="ana">
      <header className="ana__head">
        <Link to="/" className="ana__back">← Dashboard</Link>
        <h1 className="ana__title">ANALİTİK</h1>
        <span className="ana__tick">t{snapshot.tick} · sezon {snapshot.season}</span>
      </header>
      <div className="ana__body">
        {selected ? (
          <FirmDetail
            player={selected}
            snapshot={snapshot}
            tradeStats={tradeStats}
            onClose={() => setSelectedId(null)}
          />
        ) : (
          <Overview
            tuccars={tuccars}
            sanayicis={sanayicis}
            alicilar={alicilar}
            speklar={speklar}
            snapshot={snapshot}
            onSelect={setSelectedId}
          />
        )}
      </div>
    </div>
  );
}

// ─── Genel bakış ─────────────────────────────────────────────────────────────

function Overview({ tuccars, sanayicis, alicilar, speklar, snapshot, onSelect }: {
  tuccars: PlayerDto[]; sanayicis: PlayerDto[];
  alicilar: PlayerDto[]; speklar: PlayerDto[];
  snapshot: Snapshot; onSelect: (id: number) => void;
}) {
  return (
    <div className="ana__overview">
      <section className="ana__section">
        <h2 className="ana__section-title">LOJİSTİK ŞİRKETLERİ</h2>
        <div className="ana__cards">
          {tuccars.map(p => (
            <FirmCard key={p.id} player={p} snapshot={snapshot} onClick={() => onSelect(p.id)} />
          ))}
        </div>
      </section>
      <section className="ana__section">
        <h2 className="ana__section-title">SANAYİ GRUPLARI</h2>
        <div className="ana__cards">
          {sanayicis.map(p => (
            <FirmCard key={p.id} player={p} snapshot={snapshot} onClick={() => onSelect(p.id)} />
          ))}
        </div>
      </section>
      <AggregateBlock title="ALICILAR" subTitle="ortalama" players={alicilar} snapshot={snapshot} />
      <AggregateBlock title="SPEKÜLATÖRLER" subTitle="ortalama" players={speklar} snapshot={snapshot} />
    </div>
  );
}

// ─── Firma kartı ─────────────────────────────────────────────────────────────

function FirmCard({ player, snapshot, onClick }: {
  player: PlayerDto; snapshot: Snapshot; onClick: () => void;
}) {
  const color = roleColor(player.npc_kind);
  const myFarms = snapshot.private_farms.filter(f => f.owner === player.id);
  const myFabs = snapshot.factories.filter(f => f.owner === player.id);
  const myCaravans = snapshot.caravans.filter(c => c.owner === player.id);
  const myRels = snapshot.relations.filter(r => r.player_a === player.id || r.player_b === player.id);
  const avgTrust = myRels.length > 0 ? myRels.reduce((s, r) => s + r.trust_score, 0) / myRels.length : 0;
  const activeFabs = myFabs.filter(f => !f.idle).length;
  const sign = player.pnl_lira > 0 ? "pos" : player.pnl_lira < 0 ? "neg" : "";

  return (
    <button className="ana__card" onClick={onClick}>
      <div className="ana__card-head">
        <span className="ana__card-badge" style={{ color, borderColor: color }}>
          {player.npc_kind === "Tüccar" ? "TÜC" : "SAN"}
        </span>
        <span className="ana__card-name">{player.name}</span>
      </div>
      <div className="ana__card-kpis">
        <Kpi label="PnL" value={`${signedCompact(player.pnl_lira)}₺`} tone={sign} />
        <Kpi label="NAKİT" value={`${compact(player.cash_lira)}₺`} />
        {myFabs.length > 0 && <Kpi label="FAB" value={`${activeFabs}/${myFabs.length}`} />}
        {myFarms.length > 0 && <Kpi label="TARLA" value={`${myFarms.length}`} tone="accent" />}
        {myCaravans.length > 0 && <Kpi label="KERVAN" value={`${myCaravans.length}`} />}
        {avgTrust > 0 && <Kpi label="GÜVEN" value={`%${Math.round(avgTrust * 100)}`} />}
      </div>
      {myFarms.length > 0 && (
        <div className="ana__card-farms">
          {myFarms.map(f => (
            <span key={f.id} className="ana__farm-tag">
              🌾 {PRODUCT_LABEL[f.product] ?? f.product} · {CITY_LABEL[f.city] ?? f.city}
            </span>
          ))}
        </div>
      )}
    </button>
  );
}

// ─── Alıcı/Spekülatör ortalama blok ─────────────────────────────────────────

function AggregateBlock({ title, subTitle, players, snapshot }: {
  title: string; subTitle: string; players: PlayerDto[]; snapshot: Snapshot;
}) {
  if (players.length === 0) return null;
  const avgPnl = players.reduce((s, p) => s + p.pnl_lira, 0) / players.length;
  const avgCash = players.reduce((s, p) => s + p.cash_lira, 0) / players.length;
  const allRels = snapshot.relations.filter(r =>
    players.some(p => p.id === r.player_a || p.id === r.player_b)
  );
  const avgTrust = allRels.length > 0
    ? allRels.reduce((s, r) => s + r.trust_score, 0) / allRels.length : 0;

  // Hangi firmaya en çok alım/satım yapıldı
  const partnerFreq: Record<number, number> = {};
  for (const r of allRels) {
    for (const p of players) {
      const other = r.player_a === p.id ? r.player_b : r.player_b === p.id ? r.player_a : null;
      if (other != null) partnerFreq[other] = (partnerFreq[other] ?? 0) + r.trade_count;
    }
  }
  const topPartnerId = Object.entries(partnerFreq).sort((a, b) => Number(b[1]) - Number(a[1]))[0]?.[0];
  const topPartner = topPartnerId ? snapshot.leaderboard.find(p => p.id === Number(topPartnerId)) : null;

  return (
    <section className="ana__section ana__section--agg">
      <h2 className="ana__section-title">{title} <span className="ana__section-sub">{subTitle} · {players.length} oyuncu</span></h2>
      <div className="ana__agg-row">
        <Kpi label="ORT PnL" value={`${signedCompact(avgPnl)}₺`} tone={avgPnl > 0 ? "pos" : "neg"} />
        <Kpi label="ORT NAKİT" value={`${compact(avgCash)}₺`} />
        <Kpi label="ORT GÜVEN" value={avgTrust > 0 ? `%${Math.round(avgTrust * 100)}` : "—"} />
        <Kpi label="İLİŞKİ" value={`${allRels.length}`} />
        {topPartner && <Kpi label="EN ÇOK" value={topPartner.name} />}
      </div>
    </section>
  );
}

// ─── Firma detayı ────────────────────────────────────────────────────────────

function FirmDetail({ player, snapshot, tradeStats, onClose }: {
  player: PlayerDto; snapshot: Snapshot;
  tradeStats: import("../hooks/useGameSocket").PlayerTradeStats;
  onClose: () => void;
}) {
  const color = roleColor(player.npc_kind);
  const myFabs = snapshot.factories.filter(f => f.owner === player.id);
  const myFarms = snapshot.private_farms.filter(f => f.owner === player.id);
  const myCaravans = snapshot.caravans.filter(c => c.owner === player.id);
  const myRels = snapshot.relations
    .filter(r => r.player_a === player.id || r.player_b === player.id)
    .sort((a, b) => b.trust_score - a.trust_score)
    .slice(0, 12);
  const prods = topProducts(tradeStats, player.id);

  return (
    <div className="ana__detail">
      <button className="ana__back ana__detail-back" onClick={onClose}>← Geri</button>
      <div className="ana__detail-top">
        <span className="ana__card-badge" style={{ color, borderColor: color }}>{player.npc_kind}</span>
        <h2 className="ana__detail-name">{player.name}</h2>
      </div>
      <div className="ana__agg-row" style={{ marginBottom: "1.5rem" }}>
        <Kpi label="PnL" value={`${signedCompact(player.pnl_lira)}₺`} tone={player.pnl_lira > 0 ? "pos" : "neg"} />
        <Kpi label="NAKİT" value={`${lira(player.cash_lira)}₺`} />
        <Kpi label="FABRİKA" value={`${myFabs.length}`} />
        <Kpi label="TARLA" value={`${myFarms.length}`} tone={myFarms.length > 0 ? "accent" : ""} />
        <Kpi label="KERVAN" value={`${myCaravans.length}`} />
        <Kpi label="İLİŞKİ" value={`${myRels.length}`} />
      </div>

      <div className="ana__detail-cols">
        {/* Sol kolon: varlıklar + ticaret */}
        <div className="ana__detail-col">
          {myFabs.length > 0 && (
            <div className="ana__col-block">
              <h3 className="ana__col-title">FABRİKALAR</h3>
              {myFabs.map(f => (
                <div key={f.id} className={`ana__row ${f.idle ? "ana__row--dim" : ""}`}>
                  <span>{PRODUCT_LABEL[f.product] ?? f.product}</span>
                  <span className="ana__row-meta">{CITY_LABEL[f.city] ?? f.city}</span>
                  <span className={`ana__row-tag ${f.idle ? "" : "ana__row-tag--ok"}`}>
                    {f.idle ? "atıl" : `${f.pending_units}bk`}
                  </span>
                </div>
              ))}
            </div>
          )}
          {myFarms.length > 0 && (
            <div className="ana__col-block">
              <h3 className="ana__col-title">ÖZEL TARLALAR <span className="ana__col-note">münhasır ham madde</span></h3>
              {myFarms.map(f => (
                <div key={f.id} className="ana__row ana__row--highlight">
                  <span>🌾 {PRODUCT_LABEL[f.product] ?? f.product}</span>
                  <span className="ana__row-meta">{CITY_LABEL[f.city] ?? f.city}</span>
                  <span className="ana__row-tag ana__row-tag--green">sadece bana</span>
                </div>
              ))}
            </div>
          )}
          {prods.length > 0 && (
            <div className="ana__col-block">
              <h3 className="ana__col-title">SEZON TİCARETİ</h3>
              <div className="ana__trade-head">
                <span>ürün</span><span>aldı</span><span>sattı</span><span>toplam</span>
              </div>
              {prods.map(p => (
                <div key={p.product} className="ana__trade-row">
                  <span>{PRODUCT_LABEL[p.product] ?? p.product}</span>
                  <span className="gain-dim">{p.buy_qty > 0 ? compact(p.buy_qty) : "—"}</span>
                  <span className="loss-dim">{p.sell_qty > 0 ? compact(p.sell_qty) : "—"}</span>
                  <span className="num">{compact(p.buy_qty + p.sell_qty)}</span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Sağ kolon: ilişkiler */}
        <div className="ana__detail-col">
          <div className="ana__col-block">
            <h3 className="ana__col-title">GÜVEN İLİŞKİLERİ</h3>
            {myRels.length === 0 && <p className="ana__empty-small">henüz ilişki birikmedi</p>}
            {myRels.map(r => {
              const otherId = r.player_a === player.id ? r.player_b : r.player_a;
              const other = snapshot.leaderboard.find(p => p.id === otherId);
              const pct = Math.round(r.trust_score * 100);
              const barColor = pct > 60 ? "var(--gain-dim)" : pct > 30 ? "var(--accent-dim)" : "var(--line-strong)";
              return (
                <div key={`${r.player_a}-${r.player_b}`} className="ana__rel">
                  <div className="ana__rel-top">
                    <span className="ana__rel-name">{other?.name ?? `#${otherId}`}</span>
                    <span className="ana__rel-kind">{other?.npc_kind ?? "?"}</span>
                    <span className="ana__rel-count">{r.trade_count} işlem · {compact(r.total_units)} birim</span>
                  </div>
                  <div className="ana__rel-bar-wrap">
                    <div className="ana__rel-bar" style={{ width: `${pct}%`, background: barColor }} />
                    <span className="ana__rel-pct">%{pct}</span>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── KPI chip ────────────────────────────────────────────────────────────────

function Kpi({ label, value, tone = "" }: { label: string; value: string; tone?: string }) {
  return (
    <div className="ana__kpi">
      <span className="ana__kpi-l">{label}</span>
      <span className={`ana__kpi-v num ${tone ? `ana__kpi-v--${tone}` : ""}`}>{value}</span>
    </div>
  );
}
