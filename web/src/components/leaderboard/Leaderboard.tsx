import { useEffect, useRef, useState } from "react";
import type { PlayerDto, Snapshot } from "../../types";
import { lira, signedCompact } from "../../lib/format";
import { roleCode, roleColor } from "../../lib/roles";
import "./leaderboard.css";

interface Props {
  snapshot: Snapshot | null;
  prev: Snapshot | null;
}

export function Leaderboard({ snapshot, prev }: Props) {
  const rows = snapshot?.leaderboard ?? [];
  const prevMap = new Map<number, number>(
    (prev?.leaderboard ?? []).map((p) => [p.id, p.pnl_lira]),
  );
  const maxAbs = Math.max(1, ...rows.map((r) => Math.abs(r.pnl_lira)));

  return (
    <section className="lb panel">
      <div className="panel__head">
        <h2 className="panel__title">SIRALAMA</h2>
        <span className="panel__sub">{rows.length} oyuncu · PnL</span>
      </div>
      <div className="lb__cols">
        <span>#</span>
        <span>rol</span>
        <span>oyuncu</span>
        <span className="lb__r">nakit</span>
        <span className="lb__r">PnL</span>
        <span />
      </div>
      <div className="lb__body">
        {rows.map((p, i) => (
          <LeaderRow
            key={p.id}
            rank={i + 1}
            player={p}
            prevPnl={prevMap.get(p.id)}
            maxAbs={maxAbs}
          />
        ))}
        {rows.length === 0 && <div className="lb__empty">veri bekleniyor…</div>}
      </div>
    </section>
  );
}

function LeaderRow({
  rank,
  player,
  prevPnl,
  maxAbs,
}: {
  rank: number;
  player: PlayerDto;
  prevPnl: number | undefined;
  maxAbs: number;
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
    <div className={`lb__row ${flash ? `lb__row--flash-${flash}` : ""}`} data-rank={rank}>
      <span className="lb__rank num">{rank}</span>
      <span className="lb__role" style={{ color, borderColor: color }}>
        {roleCode(player.npc_kind)}
      </span>
      <span className="lb__name">{player.name}</span>
      <span className="lb__cash num">{lira(player.cash_lira)}</span>
      <span className={`lb__pnl lb__pnl--${sign} num`}>{signedCompact(player.pnl_lira)}</span>
      <span className="lb__bar">
        <span className={`lb__bar-fill lb__bar-fill--${sign}`} style={{ width: `${barW}%` }} />
      </span>
    </div>
  );
}
