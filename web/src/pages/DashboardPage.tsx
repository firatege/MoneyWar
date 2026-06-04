// Ana dashboard sayfası — mevcut App içeriği buraya taşındı.
// Gelecek sayfalar: /analytics, /firms/:id, /market/:product

import { useState } from "react";
import "../app.css";

import { useGameSocket } from "../hooks/useGameSocket";
import { SeasonHeader } from "../components/season-header/SeasonHeader";
import { TickerTape } from "../components/ticker-tape/TickerTape";
import { Leaderboard } from "../components/leaderboard/Leaderboard";
import { EventFeed } from "../components/event-feed/EventFeed";
import { PriceGrid } from "../components/price-grid/PriceGrid";
import { MarketOverview } from "../components/market-overview/MarketOverview";
import { OrderBook } from "../components/order-book/OrderBook";
import { PlayerDetail } from "../components/player-detail/PlayerDetail";
import { Footer } from "../components/footer/Footer";
import { HelpOverlay } from "../components/help/HelpOverlay";

const DEFAULT_CITY = "istanbul";
const DEFAULT_PRODUCT = "pamuk";
const INTRO_SEEN_KEY = "mw_intro_seen";

export function DashboardPage() {
  const { snapshot, prev, feed, status, history, bucketHistory, market, tradeStats, stableNews, seasons, resetSeason } =
    useGameSocket();
  const [selectedCity, setSelectedCity] = useState(DEFAULT_CITY);
  const [selectedProduct, setSelectedProduct] = useState(DEFAULT_PRODUCT);
  const [selectedPlayer, setSelectedPlayer] = useState<number | null>(null);
  const [showHelp, setShowHelp] = useState(() => localStorage.getItem(INTRO_SEEN_KEY) == null);

  const closeHelp = () => {
    setShowHelp(false);
    localStorage.setItem(INTRO_SEEN_KEY, "1");
  };

  const handleCellSelect = (city: string, product: string) => {
    setSelectedCity(city);
    setSelectedProduct(product);
    setSelectedPlayer(null);
  };

  return (
    <div className="app">
      {showHelp && <HelpOverlay onClose={closeHelp} />}
      <SeasonHeader snapshot={snapshot} status={status} onHelp={() => setShowHelp(true)} seasons={seasons} onReset={resetSeason} />
      <TickerTape news={stableNews} />
      <main className="app__grid">
        <div className="app__col app__col--left">
          <Leaderboard
            snapshot={snapshot}
            prev={prev}
            selectedId={selectedPlayer}
            onSelect={setSelectedPlayer}
          />
          <EventFeed feed={feed} />
        </div>

        <div className="app__col app__col--center">
          <PriceGrid
            snapshot={snapshot}
            bucketHistory={bucketHistory}
            selected={{ city: selectedCity, product: selectedProduct }}
            onSelect={handleCellSelect}
          />
          {selectedPlayer != null ? (
            <PlayerDetail
              playerId={selectedPlayer}
              snapshot={snapshot}
              history={history[selectedPlayer] ?? []}
              tradeStats={tradeStats}
              onClose={() => setSelectedPlayer(null)}
            />
          ) : (
            <MarketOverview market={market} snapshot={snapshot} />
          )}
        </div>

        <div className="app__col app__col--right">
          <OrderBook
            snapshot={snapshot}
            city={selectedCity}
            product={selectedProduct}
          />
        </div>
      </main>
      <Footer onHelp={() => setShowHelp(true)} />
    </div>
  );
}
