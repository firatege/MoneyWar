import { useState } from "react";
import { Link, useLocation } from "react-router-dom";
import type { ConnStatus, SeasonSummary, Snapshot } from "../../types";
import { clock, lira, signedCompact } from "../../lib/format";
import { Logo } from "../brand/Logo";
import "./season-header.css";

interface Props {
  snapshot: Snapshot | null;
  status: ConnStatus;
  onHelp: () => void;
  seasons?: SeasonSummary[];
  onReset?: () => Promise<void>;
}

const STATUS_LABEL: Record<ConnStatus, string> = {
  connecting: "BAĞLANIYOR",
  open: "CANLI",
  closed: "KOPTU",
};

export function SeasonHeader({ snapshot, status, onHelp, seasons = [], onReset }: Props) {
  const season = snapshot?.season ?? 0;
  const tick = snapshot?.tick ?? 0;
  const total = snapshot?.season_ticks ?? 90;
  const spt = snapshot?.seconds_per_tick ?? 2;
  const pct = total > 0 ? Math.min(100, (tick / total) * 100) : 0;
  const remaining = clock((total - tick) * spt);
  const [showHistory, setShowHistory] = useState(false);
  const [resetting, setResetting] = useState(false);
  const loc = useLocation();
  const onAnalytics = loc.pathname.startsWith("/analytics");

  const handleReset = async () => {
    if (!onReset) return;
    setResetting(true);
    try { await onReset(); } finally { setResetting(false); }
  };

  return (
    <>
      <header className="hdr">
        <div className="hdr__brand">
          <Logo size={22} className="hdr__logo" />
          <span className="hdr__wordmark">MONEYWAR</span>
          <span className={`hdr__status hdr__status--${status}`}>
            <i className="hdr__dot" />
            {STATUS_LABEL[status]}
          </span>
          <button className="hdr__help" onClick={onHelp} title="nasıl çalışır">
            nasıl çalışır?
          </button>
          <Link
            className={`hdr__help hdr__nav-link ${onAnalytics ? "hdr__nav-link--active" : ""}`}
            to={onAnalytics ? "/" : "/analytics"}
          >
            {onAnalytics ? "← dashboard" : "analitik →"}
          </Link>
          {seasons.length > 0 && (
            <button className="hdr__help" onClick={() => setShowHistory(true)} title="geçmiş sezonlar">
              geçmiş
            </button>
          )}
          {onReset && (
            <button
              className="hdr__help hdr__reset"
              onClick={() => void handleReset()}
              disabled={resetting}
              title="sezonu sıfırla"
            >
              {resetting ? "…" : "↺ sıfırla"}
            </button>
          )}
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

      {showHistory && (
        <SeasonHistoryModal seasons={seasons} onClose={() => setShowHistory(false)} />
      )}
    </>
  );
}

function SeasonHistoryModal({ seasons, onClose }: { seasons: SeasonSummary[]; onClose: () => void }) {
  return (
    <div className="sh-overlay" onClick={onClose}>
      <div className="sh-modal" onClick={e => e.stopPropagation()}>
        <div className="sh-modal__head">
          <span className="sh-modal__title">SEZON GEÇMİŞİ</span>
          <button className="sh-modal__close" onClick={onClose}>✕</button>
        </div>
        <div className="sh-modal__body">
          {seasons.length === 0 && (
            <p className="sh-modal__empty">henüz tamamlanan sezon yok</p>
          )}
          {seasons.map(s => (
            <SeasonBlock key={s.season} summary={s} />
          ))}
        </div>
      </div>
    </div>
  );
}

function SeasonBlock({ summary }: { summary: SeasonSummary }) {
  const winner = summary.top[0];
  return (
    <div className="sh-season">
      <div className="sh-season__head">
        <span className="sh-season__num">Sezon {summary.season}</span>
        <span className="sh-season__ticks">{summary.ticks_completed} tick</span>
        {winner && (
          <span className="sh-season__winner">
            🥇 {winner.name}
            <span className="sh-season__pnl gain">{lira(winner.pnl_lira)}₺</span>
          </span>
        )}
      </div>
      <div className="sh-season__table">
        {summary.top.slice(0, 5).map(e => (
          <div key={e.id} className="sh-season__row">
            <span className="sh-season__rank">{e.rank}</span>
            <span className="sh-season__name">{e.name}</span>
            <span className="sh-season__kind">{e.npc_kind ?? "insan"}</span>
            <span className={`sh-season__val num ${e.pnl_lira >= 0 ? "gain" : "loss"}`}>
              {signedCompact(e.pnl_lira)}₺
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
