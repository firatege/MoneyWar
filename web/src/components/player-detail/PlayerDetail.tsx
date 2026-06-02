import type { PlayerDto, Snapshot } from "../../types";
import type { PnlPoint } from "../../hooks/useGameSocket";
import { lira, signedCompact } from "../../lib/format";
import { roleColor } from "../../lib/roles";
import { TrendChart } from "../trend-chart/TrendChart";
import "./player-detail.css";

interface Props {
  playerId: number;
  snapshot: Snapshot | null;
  history: PnlPoint[];
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

export function PlayerDetail({ playerId, snapshot, history, onClose }: Props) {
  const player: PlayerDto | undefined = snapshot?.leaderboard.find(
    (p) => p.id === playerId,
  );
  const rank =
    (snapshot?.leaderboard.findIndex((p) => p.id === playerId) ?? -1) + 1;
  const factories = (snapshot?.factories ?? []).filter((f) => f.owner === playerId);
  const caravans = (snapshot?.caravans ?? []).filter((c) => c.owner === playerId);

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
        <Stat
          label="PnL"
          value={`${signedCompact(player.pnl_lira)} ₺`}
          tone={sign}
        />
        <Stat label="FABRİKA" value={`${factories.length}`} />
        <Stat label="KERVAN" value={`${caravans.length}`} />
      </div>

      <div className="pd__chart-wrap">
        <div className="pd__chart-label">PnL SEYRİ (sezon)</div>
        <TrendChart points={trendPoints} baseline={0} emptyText="PnL geçmişi birikiyor…" />
      </div>

      <div className="pd__assets">
        <div className="pd__asset-col">
          <div className="pd__asset-title">FABRİKALAR</div>
          {factories.length === 0 && <div className="pd__asset-empty">yok</div>}
          {factories.map((f) => (
            <div key={f.id} className={`pd__asset ${f.idle ? "pd__asset--idle" : ""}`}>
              <span>
                {PRODUCT_LABEL[f.product] ?? f.product} · {CITY_LABEL[f.city] ?? f.city}
              </span>
              <span className="pd__asset-meta num">
                {f.idle ? "ATIL" : `${f.pending_units} bk`}
              </span>
            </div>
          ))}
        </div>
        <div className="pd__asset-col">
          <div className="pd__asset-title">KERVANLAR</div>
          {caravans.length === 0 && <div className="pd__asset-empty">yok</div>}
          {caravans.map((c) => (
            <div key={c.id} className="pd__asset">
              <span>#{c.id}</span>
              <span className="pd__asset-meta num">
                {c.idle
                  ? `demirli · ${CITY_LABEL[c.current_city ?? ""] ?? "—"}`
                  : `yolda · ${c.cargo_units} bk`}
              </span>
            </div>
          ))}
        </div>
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
