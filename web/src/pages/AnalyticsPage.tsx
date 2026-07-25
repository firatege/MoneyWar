import { useNavigate } from "react-router-dom";
import { AnalyticsLayout } from "./AnalyticsLayout";

import "./analytics.css";
import { useGameSocket } from "../hooks/useGameSocket";
import { compact, signedCompact } from "../lib/format";
import { roleColor } from "../lib/roles";
import type { PlayerDto, Snapshot } from "../types";
import "./pages.css";
import { PRODUCT_LABEL } from "../lib/catalog";

const CITY_LABEL: Record<string, string> = {
  istanbul: "İst", ankara: "Ank", izmir: "İzm", bursa: "Bur", konya: "Kon",
};

export function AnalyticsPage() {
  const { snapshot } = useGameSocket();
  const navigate = useNavigate();

  if (!snapshot) {
    return (
      <AnalyticsLayout>
        <div className="ana">
          <div className="ana__empty">bağlantı bekleniyor…</div>
        </div>
      </AnalyticsLayout>
    );
  }

  const players = snapshot.leaderboard;
  const tuccars = players.filter(p => p.npc_kind === "Tüccar");
  const sanayicis = players.filter(p => p.npc_kind === "Sanayici");
  const alicilar = players.filter(p => p.npc_kind === "Alıcı");
  const speklar = players.filter(p => p.npc_kind === "Spekülatör");

  return (
    <AnalyticsLayout>
      <div className="ana">
        <div className="ana__body">
          <Overview
            tuccars={tuccars}
            sanayicis={sanayicis}
            alicilar={alicilar}
            speklar={speklar}
            snapshot={snapshot}
            onSelect={(id) => void navigate(`/analytics/firm/${id}`)}
          />
        </div>
      </div>
    </AnalyticsLayout>
  );
}

// ─── Genel bakış ─────────────────────────────────────────────────────────────

function Overview({ tuccars, sanayicis, alicilar, speklar, snapshot, onSelect }: {
  tuccars: PlayerDto[]; sanayicis: PlayerDto[];
  alicilar: PlayerDto[]; speklar: PlayerDto[];
  snapshot: Snapshot;
  onSelect: (id: number) => void;
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

// ─── KPI chip ────────────────────────────────────────────────────────────────

function Kpi({ label, value, tone = "" }: { label: string; value: string; tone?: string }) {
  return (
    <div className="ana__kpi">
      <span className="ana__kpi-l">{label}</span>
      <span className={`ana__kpi-v num ${tone ? `ana__kpi-v--${tone}` : ""}`}>{value}</span>
    </div>
  );
}
