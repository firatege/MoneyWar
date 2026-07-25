import { Link, useLocation } from "react-router-dom";
import { useGameSocket } from "../hooks/useGameSocket";
import { roleColor } from "../lib/roles";
import { signedCompact } from "../lib/format";
import { PRODUCT_LABEL } from "../lib/catalog";

const CITY_SHORT: Record<string, string> = {
  istanbul: "İST", ankara: "ANK", izmir: "İZM", bursa: "BUR", konya: "KON",
};

interface Props {
  children: React.ReactNode;
}

export function AnalyticsLayout({ children }: Props) {
  const { snapshot } = useGameSocket();
  const loc = useLocation();

  const tuccars = snapshot?.leaderboard.filter(p => p.npc_kind === "Tüccar") ?? [];
  const sanayicis = snapshot?.leaderboard.filter(p => p.npc_kind === "Sanayici") ?? [];
  const mamulBuckets = snapshot?.prices.filter(p => !p.is_raw && p.last_lira != null) ?? [];

  return (
    <div className="al">
      {/* Sidebar */}
      <aside className="al__sidebar">
        <div className="al__sidebar-head">
          <Link to="/" className="al__home">← MW</Link>
          <span className="al__tick">
            {snapshot ? `t${snapshot.tick}` : "—"}
          </span>
        </div>

        {/* Nav links */}
        <nav className="al__nav">
          <Link to="/analytics" className={`al__nav-item ${loc.pathname === "/analytics" ? "al__nav-item--active" : ""}`}>
            <span className="al__nav-icon">◈</span>
            <span>Genel</span>
          </Link>
          <Link to="/analytics/market" className={`al__nav-item ${loc.pathname === "/analytics/market" ? "al__nav-item--active" : ""}`}>
            <span className="al__nav-icon">◉</span>
            <span>Piyasa</span>
          </Link>
        </nav>

        {/* Firma listesi */}
        <div className="al__section-label">LOJİSTİK</div>
        {tuccars.map(p => (
          <Link key={p.id} to={`/analytics/firm/${p.id}`}
            className={`al__firm ${loc.pathname === `/analytics/firm/${p.id}` ? "al__firm--active" : ""}`}>
            <span className="al__firm-dot" style={{ background: roleColor(p.npc_kind) }} />
            <span className="al__firm-name">{p.name}</span>
            <span className={`al__firm-pnl ${p.pnl_lira >= 0 ? "al__firm-pnl--pos" : "al__firm-pnl--neg"}`}>
              {signedCompact(p.pnl_lira)}
            </span>
          </Link>
        ))}

        <div className="al__section-label">SANAYİ</div>
        {sanayicis.map(p => (
          <Link key={p.id} to={`/analytics/firm/${p.id}`}
            className={`al__firm ${loc.pathname === `/analytics/firm/${p.id}` ? "al__firm--active" : ""}`}>
            <span className="al__firm-dot" style={{ background: roleColor(p.npc_kind) }} />
            <span className="al__firm-name">{p.name}</span>
            <span className={`al__firm-pnl ${p.pnl_lira >= 0 ? "al__firm-pnl--pos" : "al__firm-pnl--neg"}`}>
              {signedCompact(p.pnl_lira)}
            </span>
          </Link>
        ))}

        {/* Mamul bucket listesi */}
        <div className="al__section-label">BUCKETLAR</div>
        {mamulBuckets.slice(0, 15).map(c => {
          const path = `/analytics/bucket/${c.city}/${c.product}`;
          const pct = c.baseline_lira > 0 && c.last_lira
            ? ((c.last_lira - c.baseline_lira) / c.baseline_lira * 100)
            : 0;
          return (
            <Link key={`${c.city}-${c.product}`} to={path}
              className={`al__bucket ${loc.pathname === path ? "al__bucket--active" : ""}`}>
              <span className="al__bucket-name">{PRODUCT_LABEL[c.product] ?? c.product}</span>
              <span className="al__bucket-city">{CITY_SHORT[c.city] ?? c.city}</span>
              <span className={`al__bucket-pct ${pct >= 0 ? "al__bucket-pct--up" : "al__bucket-pct--dn"}`}>
                {pct >= 0 ? "+" : ""}{pct.toFixed(0)}%
              </span>
            </Link>
          );
        })}
      </aside>

      {/* İçerik */}
      <main className="al__content">
        {children}
      </main>
    </div>
  );
}
