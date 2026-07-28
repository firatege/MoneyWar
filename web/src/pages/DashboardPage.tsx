import { useEffect, useState } from "react";
import { useGameSocket } from "../hooks/useGameSocket";
import { SeasonHeader } from "../components/season-header/SeasonHeader";
import { Rankings } from "../components/rankings/Rankings";
import { NetworkMap } from "../components/network-map/NetworkMap";
import { LayeredFeed } from "../components/event-feed/LayeredFeed";
import { ChartStrip } from "../components/chart-strip/ChartStrip";
import { CityPanel } from "./CityPanel";
import { FirmPanel } from "./FirmPanel";
import { FactoryPanel } from "./FactoryPanel";
import { BucketPanel } from "./BucketPanel";
import { RelationsPanel } from "./RelationsPanel";
import { MarketGridPanel } from "./MarketGridPanel";
import { Footer } from "../components/footer/Footer";
import { HelpOverlay } from "../components/help/HelpOverlay";
import "../app.css";

const INTRO_SEEN_KEY = "mw_intro_seen";

/**
 * Ana izleyici ekranı.
 *
 * Üç kademeli inceleme: şehir → firma → fabrika. Orta sütun kademeye göre
 * içerik değiştirir; harita ilk kademede kalır.
 */
export type Focus =
  | { kind: "none" }
  | { kind: "city"; slug: string }
  | { kind: "firm"; id: number }
  | { kind: "factory"; id: number }
  | { kind: "relations" }
  | { kind: "grid" }
  | { kind: "bucket"; city: string; product: string };

export function DashboardPage() {
  const { snapshot, prev, feed, status, market, history, bucketHistory, seasons, resetSeason } =
    useGameSocket();
  const [focus, setFocus] = useState<Focus>({ kind: "none" });
  const [showHelp, setShowHelp] = useState(false);

  // İlk ziyarette tanıtımı otomatik aç.
  useEffect(() => {
    if (localStorage.getItem(INTRO_SEEN_KEY) == null) setShowHelp(true);
  }, []);

  const closeHelp = () => {
    setShowHelp(false);
    localStorage.setItem(INTRO_SEEN_KEY, "1");
  };

  // Escape haritaya döner — detaya girip sıkışmak olmasın.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !showHelp) setFocus({ kind: "none" });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showHelp]);

  const tick = snapshot?.tick ?? 0;
  const selectedCity = focus.kind === "city" ? focus.slug : null;

  return (
    <div className="app">
      {showHelp && <HelpOverlay onClose={closeHelp} />}
      <SeasonHeader
        snapshot={snapshot}
        status={status}
        onHelp={() => setShowHelp(true)}
        onRelations={() => setFocus({ kind: "relations" })}
        onGrid={() => setFocus({ kind: "grid" })}
        seasons={seasons}
        onReset={resetSeason}
      />

      <main className="app__grid">
        {/* Sol — kim kazanıyor */}
        <div className="app__col app__col--left">
          <Rankings
            snapshot={snapshot}
            prev={prev}
            selectedId={focus.kind === "firm" ? focus.id : null}
            onSelect={(id) => setFocus({ kind: "firm", id })}
          />
        </div>

        {/* Orta — harita ve üstüne binen detay kademeleri */}
        <div className="app__col app__col--center">
          {(focus.kind === "none" || focus.kind === "city") && (
            <NetworkMap
              snapshot={snapshot}
              compact={focus.kind === "city"}
              selected={selectedCity}
              onSelect={(slug) =>
                setFocus(selectedCity === slug ? { kind: "none" } : { kind: "city", slug })
              }
            />
          )}

          {focus.kind === "city" && (
            <CityPanel
              slug={focus.slug}
              tick={tick}
              snapshot={snapshot}
              bucketHistory={bucketHistory}
              onClose={() => setFocus({ kind: "none" })}
              onSelectFirm={(id) => setFocus({ kind: "firm", id })}
              onSelectFactory={(id) => setFocus({ kind: "factory", id })}
            />
          )}
          {focus.kind === "firm" && (
            <FirmPanel
              id={focus.id}
              tick={tick}
              pnlHistory={history[focus.id] ?? []}
              onClose={() => setFocus({ kind: "none" })}
              onSelectFactory={(id) => setFocus({ kind: "factory", id })}
              onSelectFirm={(id) => setFocus({ kind: "firm", id })}
            />
          )}
          {focus.kind === "factory" && (
            <FactoryPanel
              id={focus.id}
              tick={tick}
              onClose={() => setFocus({ kind: "none" })}
              onSelectFirm={(id) => setFocus({ kind: "firm", id })}
            />
          )}
          {focus.kind === "grid" && (
            <MarketGridPanel
              snapshot={snapshot}
              bucketHistory={bucketHistory}
              onClose={() => setFocus({ kind: "none" })}
              onOpenBucket={(city, product) => setFocus({ kind: "bucket", city, product })}
            />
          )}
          {focus.kind === "bucket" && (
            <BucketPanel
              city={focus.city}
              product={focus.product}
              tick={tick}
              bucketHistory={bucketHistory}
              onClose={() => setFocus({ kind: "grid" })}
              onSelectCity={(slug) => setFocus({ kind: "city", slug })}
            />
          )}
          {focus.kind === "relations" && (
            <RelationsPanel
              tick={tick}
              onClose={() => setFocus({ kind: "none" })}
              onSelectFirm={(id) => setFocus({ kind: "firm", id })}
            />
          )}
        </div>

        {/* Sağ — ne oluyor */}
        <div className="app__col app__col--right">
          <LayeredFeed
            feed={feed}
            tick={tick}
            onSelectFirm={(id) => setFocus({ kind: "firm", id })}
          />
        </div>
      </main>

      <ChartStrip snapshot={snapshot} market={market} />
      <Footer onHelp={() => setShowHelp(true)} />
    </div>
  );
}
