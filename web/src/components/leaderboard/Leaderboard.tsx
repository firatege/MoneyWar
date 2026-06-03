import { useEffect, useRef, useState } from "react";
import type { PlayerDto, Snapshot } from "../../types";
import { signedCompact } from "../../lib/format";
import { roleCode, roleColor } from "../../lib/roles";
import "./leaderboard.css";

interface Props {
  snapshot: Snapshot | null;
  prev: Snapshot | null;
  selectedId: number | null;
  onSelect: (id: number) => void;
}

// Sadece bu roller sıralamada gösterilir (kullanıcı isteği).
const SHOWN_KINDS = new Set(["Sanayici", "Tüccar"]);

function filterRanked(snap: Snapshot | null): PlayerDto[] {
  return (snap?.leaderboard ?? []).filter(
    (p) => p.npc_kind != null && SHOWN_KINDS.has(p.npc_kind),
  );
}

export function Leaderboard({ snapshot, prev, selectedId, onSelect }: Props) {
  const rows = filterRanked(snapshot);

  // Önceki sıraları (filtrelenmiş liste içinde) hesapla → sıra değişimi.
  const prevRank = new Map<number, number>();
  filterRanked(prev).forEach((p, i) => prevRank.set(p.id, i));

  const maxAbs = Math.max(1, ...rows.map((r) => Math.abs(r.pnl_lira)));

  return (
    <section className="lb panel">
      <div className="panel__head">
        <h2 className="panel__title">SIRALAMA</h2>
        <span className="panel__sub">Sanayici · Tüccar</span>
      </div>
      <div className="lb__cols">
        <span>#</span>
        <span>rol</span>
        <span>oyuncu</span>
        <span className="lb__r">PnL</span>
        <span />
      </div>
      <div className="lb__body">
        {rows.map((p, i) => {
          const pr = prevRank.get(p.id);
          const rankDelta = pr === undefined ? 0 : pr - i; // + = yükseldi
          return (
            <LeaderRow
              key={p.id}
              rank={i + 1}
              rankDelta={rankDelta}
              player={p}
              prevPnl={prev?.leaderboard.find((x) => x.id === p.id)?.pnl_lira}
              maxAbs={maxAbs}
              selected={selectedId === p.id}
              onSelect={() => onSelect(p.id)}
            />
          );
        })}
        {rows.length === 0 && <div className="lb__empty">veri bekleniyor…</div>}
      </div>
    </section>
  );
}

function LeaderRow({
  rank,
  rankDelta,
  player,
  prevPnl,
  maxAbs,
  selected,
  onSelect,
}: {
  rank: number;
  rankDelta: number;
  player: PlayerDto;
  prevPnl: number | undefined;
  maxAbs: number;
  selected: boolean;
  onSelect: () => void;
}) {
  const [flash, setFlash] = useState<"up" | "down" | null>(null);
  const lastPnl = useRef(player.pnl_lira);

  useEffect(() => {
    if (prevPnl !== undefined && player.pnl_lira !== lastPnl.current) {
      setFlash(player.pnl_lira > lastPnl.current ? "up" : "down");
      const t = window.setTimeout(() => setFlash(null), 700);
      lastPnl.current = player.pnl_lira;
      return () => window.clearTimeout(t);
    }
    lastPnl.current = player.pnl_lira;
  }, [player.pnl_lira, prevPnl]);

  const color = roleColor(player.npc_kind);
  const sign = player.pnl_lira > 0 ? "pos" : player.pnl_lira < 0 ? "neg" : "zero";
  const barW = (Math.abs(player.pnl_lira) / maxAbs) * 100;

  return (
    <button
      className={`lb__row ${flash ? `lb__row--flash-${flash}` : ""}${
        selected ? " lb__row--selected" : ""
      }`}
      data-rank={rank}
      onClick={onSelect}
    >
      <span className="lb__rank-cell">
        <span className="lb__rank num">{rank}</span>
        <RankDelta delta={rankDelta} />
      </span>
      <span className="lb__role" style={{ color, borderColor: color }}>
        {roleCode(player.npc_kind)}
      </span>
      <span className="lb__name">{player.name}</span>
      <span className={`lb__pnl lb__pnl--${sign} num`}>{signedCompact(player.pnl_lira)}</span>
      <span className="lb__bar">
        <span className={`lb__bar-fill lb__bar-fill--${sign}`} style={{ width: `${barW}%` }} />
      </span>
    </button>
  );
}

/** Sıra değişimi göstergesi: ▲n yükseldi, ▼n düştü, — sabit. */
function RankDelta({ delta }: { delta: number }) {
  if (delta > 0) {
    return <span className="lb__delta lb__delta--up">▲{delta}</span>;
  }
  if (delta < 0) {
    return <span className="lb__delta lb__delta--down">▼{-delta}</span>;
  }
  return <span className="lb__delta lb__delta--flat">·</span>;
}
