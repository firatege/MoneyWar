import type { ConnStatus, Snapshot } from "../../types";
import { clock } from "../../lib/format";
import "./season-header.css";

interface Props {
  snapshot: Snapshot | null;
  status: ConnStatus;
}

const STATUS_LABEL: Record<ConnStatus, string> = {
  connecting: "BAĞLANIYOR",
  open: "CANLI",
  closed: "KOPTU",
};

export function SeasonHeader({ snapshot, status }: Props) {
  const season = snapshot?.season ?? 0;
  const tick = snapshot?.tick ?? 0;
  const total = snapshot?.season_ticks ?? 90;
  const spt = snapshot?.seconds_per_tick ?? 2;
  const pct = total > 0 ? Math.min(100, (tick / total) * 100) : 0;
  const remaining = clock((total - tick) * spt);

  return (
    <header className="hdr">
      <div className="hdr__brand">
        <span className="hdr__wordmark">MONEYWAR</span>
        <span className={`hdr__status hdr__status--${status}`}>
          <i className="hdr__dot" />
          {STATUS_LABEL[status]}
        </span>
      </div>

      <div className="hdr__season">
        <span className="hdr__season-label">SEZON</span>
        <span className="hdr__season-num serif">{season || "—"}</span>
      </div>

      <div className="hdr__progress">
        <div className="hdr__progress-meta">
          <span className="hdr__tick">
            TICK <b>{tick.toString().padStart(2, "0")}</b>
            <span className="hdr__tick-sep">/</span>
            {total}
          </span>
          <span className="hdr__countdown">
            sezon sonu <b>{remaining}</b>
          </span>
        </div>
        <div className="hdr__track" role="progressbar" aria-valuenow={tick} aria-valuemax={total}>
          <div className="hdr__fill" style={{ width: `${pct}%` }} />
        </div>
      </div>
    </header>
  );
}
