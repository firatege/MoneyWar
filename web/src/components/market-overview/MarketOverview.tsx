import type { Snapshot } from "../../types";
import type { MarketPoint } from "../../hooks/useGameSocket";
import { TrendChart } from "../trend-chart/TrendChart";
import "./market-overview.css";

interface Props {
  market: MarketPoint[];
  snapshot: Snapshot | null;
}

export function MarketOverview({ market, snapshot }: Props) {
  const latest = market.at(-1);
  const indexPoints = market.map((m) => ({ tick: m.tick, value: m.index }));
  const volumePoints = market.map((m) => ({ tick: m.tick, value: m.volume }));

  const activeFactories = (snapshot?.factories ?? []).filter((f) => !f.idle).length;
  const idleFactories = (snapshot?.factories ?? []).filter((f) => f.idle).length;
  const caravans = snapshot?.caravans.length ?? 0;

  const idx = latest?.index ?? 100;
  const idxTone = idx > 100.5 ? "up" : idx < 99.5 ? "down" : "flat";

  return (
    <div className="mo">
      <div className="mo__head">
        <h2 className="mo__title">PİYASA GENELİ</h2>
        <div className="mo__stats">
          <Stat label="ENDEKS" value={idx.toFixed(1)} tone={idxTone} />
          <Stat label="HAM" value={(latest?.rawIndex ?? 100).toFixed(1)} />
          <Stat label="MAMUL" value={(latest?.finIndex ?? 100).toFixed(1)} />
          <Stat label="HACİM" value={`${latest?.volume ?? 0}`} />
          <Stat label="FABRİKA" value={`${activeFactories}/${activeFactories + idleFactories}`} />
          <Stat label="KERVAN" value={`${caravans}`} />
        </div>
      </div>

      <div className="mo__charts">
        <div className="mo__block mo__block--index">
          <div className="mo__block-label">
            PİYASA ENDEKSİ <span className="mo__block-note">100 = baz fiyat</span>
          </div>
          <TrendChart
            points={indexPoints}
            baseline={100}
            emptyText="endeks birikiyor…"
          />
        </div>
        <div className="mo__block mo__block--volume">
          <div className="mo__block-label">
            İŞLEM HACMİ <span className="mo__block-note">tick başına eşleşen birim</span>
          </div>
          {/* baseline=-1: hacim her zaman baseline'ın üstünde → hep yeşil */}
          <TrendChart points={volumePoints} baseline={-1} emptyText="hacim birikiyor…" />
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
  tone?: "up" | "down" | "flat";
}) {
  return (
    <span className="mo__stat">
      <span className="mo__stat-l">{label}</span>
      <span className={`mo__stat-v num ${tone ? `mo__stat-v--${tone}` : ""}`}>{value}</span>
    </span>
  );
}
