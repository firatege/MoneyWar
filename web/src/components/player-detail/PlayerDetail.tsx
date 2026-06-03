import type { PlayerDto, Snapshot, RelationDto } from "../../types";
import type { PnlPoint, PlayerTradeStats } from "../../hooks/useGameSocket";
import { lira, signedCompact, compact } from "../../lib/format";
import { roleColor } from "../../lib/roles";
import { TrendChart } from "../trend-chart/TrendChart";
import { topProducts } from "../../lib/derive";
import "./player-detail.css";

interface Props {
  playerId: number;
  snapshot: Snapshot | null;
  history: PnlPoint[];
  tradeStats: PlayerTradeStats;
  onClose: () => void;
}

const CITY_LABEL: Record<string, string> = {
  istanbul: "İstanbul",
  ankara: "Ankara",
  izmir: "İzmir",
  bursa: "Bursa",
  konya: "Konya",
};
const PRODUCT_LABEL: Record<string, string> = {
  pamuk: "Pamuk",
  bugday: "Buğday",
  zeytin: "Zeytin",
  kumas: "Kumaş",
  un: "Un",
  zeytinyagi: "Zeytinyağı",
};

export function PlayerDetail({ playerId, snapshot, history, tradeStats, onClose }: Props) {
  const player: PlayerDto | undefined = snapshot?.leaderboard.find(
    (p) => p.id === playerId,
  );
  const rank =
    (snapshot?.leaderboard.findIndex((p) => p.id === playerId) ?? -1) + 1;
  const factories = (snapshot?.factories ?? []).filter((f) => f.owner === playerId);
  const caravans = (snapshot?.caravans ?? []).filter((c) => c.owner === playerId);
  const privateFarms = (snapshot?.private_farms ?? []).filter((f) => f.owner === playerId);
  const products = topProducts(tradeStats, playerId);
  // Güven ilişkileri
  const myRels: RelationDto[] = (snapshot?.relations ?? [])
    .filter(r => r.player_a === playerId || r.player_b === playerId)
    .sort((a, b) => b.trust_score - a.trust_score)
    .slice(0, 5);
  const isTuccar = player?.npc_kind === "Tüccar";
  const isSanayici = player?.npc_kind === "Sanayici";

  if (!player) {
    return (
      <div className="pd">
        <div className="pd__empty">oyuncu bulunamadı</div>
      </div>
    );
  }

  const color = roleColor(player.npc_kind);
  const sign = player.pnl_lira > 0 ? "pos" : player.pnl_lira < 0 ? "neg" : "zero";
  const trendPoints = history.map((h) => ({ tick: h.tick, value: h.pnl }));

  return (
    <div className="pd">
      <div className="pd__head">
        <div className="pd__id">
          <span className="pd__role" style={{ color, borderColor: color }}>
            {player.npc_kind ?? "—"}
          </span>
          <span className="pd__name">{player.name}</span>
          <span className="pd__rank">#{rank}</span>
        </div>
        <button className="pd__close" onClick={onClose} title="kapat">
          ← piyasaya dön
        </button>
      </div>

      <div className="pd__stats">
        <Stat label="NAKİT" value={`${lira(player.cash_lira)} ₺`} />
        <Stat label="PnL" value={`${signedCompact(player.pnl_lira)} ₺`} tone={sign} />
        {isSanayici && <Stat label="FABRİKA" value={`${factories.length}`} />}
        {isSanayici && privateFarms.length > 0 && <Stat label="TARLA" value={`${privateFarms.length}`} />}
        {isTuccar && <Stat label="KERVAN" value={`${caravans.length}`} />}
        {myRels.length > 0 && (
          <Stat label="GÜVEN" value={`%${Math.round(myRels[0].trust_score * 100)}`} />
        )}
      </div>

      <div className="pd__chart-wrap">
        <div className="pd__chart-label">PnL SEYRİ (sezon)</div>
        <TrendChart points={trendPoints} baseline={0} emptyText="PnL geçmişi birikiyor…" />
      </div>

      {products.length > 0 && (
        <div className="pd__trades">
          <div className="pd__asset-title">SEZON İŞLEMLERİ</div>
          <div className="pd__trade-cols">
            <span className="pd__trade-h">ürün</span>
            <span className="pd__trade-h pd__r">aldı</span>
            <span className="pd__trade-h pd__r">sattı</span>
            <span className="pd__trade-h pd__r">toplam</span>
          </div>
          {products.map((p) => (
            <div key={p.product} className="pd__trade-row">
              <span>{PRODUCT_LABEL[p.product] ?? p.product}</span>
              <span className="pd__r gain-dim">{p.buy_qty > 0 ? compact(p.buy_qty) : "—"}</span>
              <span className="pd__r loss-dim">{p.sell_qty > 0 ? compact(p.sell_qty) : "—"}</span>
              <span className="pd__r num">{compact(p.buy_qty + p.sell_qty)}</span>
            </div>
          ))}
        </div>
      )}

      <div className="pd__assets">
        {/* Sanayici: fabrikalar + özel tarlalar */}
        {isSanayici && (
          <div className="pd__asset-col">
            <div className="pd__asset-title">FABRİKALAR</div>
            {factories.length === 0 && <div className="pd__asset-empty">yok</div>}
            {factories.map((f) => (
              <div key={f.id} className={`pd__asset ${f.idle ? "pd__asset--idle" : ""}`}>
                <span>{PRODUCT_LABEL[f.product] ?? f.product} · {CITY_LABEL[f.city] ?? f.city}</span>
                <span className="pd__asset-meta num">{f.idle ? "ATIL" : `${f.pending_units}bk`}</span>
              </div>
            ))}
          </div>
        )}
        {isSanayici && privateFarms.length > 0 && (
          <div className="pd__asset-col">
            <div className="pd__asset-title">ÖZEL TARLALAR</div>
            {privateFarms.map((f) => (
              <div key={f.id} className="pd__asset">
                <span>🌾 {PRODUCT_LABEL[f.product] ?? f.product}</span>
                <span className="pd__asset-meta">{CITY_LABEL[f.city] ?? f.city}</span>
              </div>
            ))}
          </div>
        )}
        {/* Tüccar: kervanlar */}
        {isTuccar && (
          <div className="pd__asset-col">
            <div className="pd__asset-title">KERVANLAR</div>
            {caravans.length === 0 && <div className="pd__asset-empty">yok</div>}
            {caravans.map((c) => (
              <div key={c.id} className="pd__asset">
                <span>#{c.id}</span>
                <span className="pd__asset-meta num">
                  {c.idle ? `demirli · ${CITY_LABEL[c.current_city ?? ""] ?? "—"}` : `yolda · ${c.cargo_units}bk`}
                </span>
              </div>
            ))}
          </div>
        )}
        {/* Güven ilişkileri */}
        {myRels.length > 0 && (
          <div className="pd__asset-col">
            <div className="pd__asset-title">EN GÜÇLÜ BAĞLAR</div>
            {myRels.map(r => {
              const otherId = r.player_a === playerId ? r.player_b : r.player_a;
              const other = snapshot?.leaderboard.find(p => p.id === otherId);
              const pct = Math.round(r.trust_score * 100);
              return (
                <div key={`${r.player_a}-${r.player_b}`} className="pd__asset">
                  <span>{other?.name ?? `#${otherId}`}</span>
                  <span className="pd__asset-meta num" style={{ color: pct > 50 ? "var(--gain-dim)" : "var(--text-faint)" }}>
                    %{pct}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "pos" | "neg" | "zero";
}) {
  return (
    <div className="pd__stat">
      <span className="pd__stat-l">{label}</span>
      <span className={`pd__stat-v num ${tone ? `pd__stat-v--${tone}` : ""}`}>
        {value}
      </span>
    </div>
  );
}
