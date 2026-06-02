import { useState } from "react";
import { useGameSocket } from "./hooks/useGameSocket";
import { useSeriesCache } from "./hooks/useSeriesCache";
import { SeasonHeader } from "./components/season-header/SeasonHeader";
import { TickerTape } from "./components/ticker-tape/TickerTape";
import { Leaderboard } from "./components/leaderboard/Leaderboard";
import { EventFeed } from "./components/event-feed/EventFeed";
import { PriceGrid } from "./components/price-grid/PriceGrid";
import { MarketChart } from "./components/market-chart/MarketChart";
import { OrderBook } from "./components/order-book/OrderBook";
import { PlayerDetail } from "./components/player-detail/PlayerDetail";
import "./app.css";

const DEFAULT_CITY = "istanbul";
const DEFAULT_PRODUCT = "pamuk";

export default function App() {
  const { snapshot, prev, feed, status, history } = useGameSocket();
  const [selectedCity, setSelectedCity] = useState(DEFAULT_CITY);
  const [selectedProduct, setSelectedProduct] = useState(DEFAULT_PRODUCT);
  const [selectedPlayer, setSelectedPlayer] = useState<number | null>(null);

  const points = useSeriesCache(selectedCity, selectedProduct, snapshot);
  const selectedCell =
    snapshot?.prices.find(
      (p) => p.city === selectedCity && p.product === selectedProduct,
    ) ?? null;

  const handleCellSelect = (city: string, product: string) => {
    setSelectedCity(city);
    setSelectedProduct(product);
    setSelectedPlayer(null); // piyasaya dön
  };

  return (
    <div className="app">
      <SeasonHeader snapshot={snapshot} status={status} />
      <TickerTape snapshot={snapshot} prev={prev} />
      <main className="app__grid">
        {/* Sol sütun: sıralama + akış */}
        <div className="app__col app__col--left">
          <Leaderboard
            snapshot={snapshot}
            prev={prev}
            selectedId={selectedPlayer}
            onSelect={setSelectedPlayer}
          />
          <EventFeed feed={feed} />
        </div>

        {/* Orta: fiyat ızgarası + grafik / oyuncu detayı */}
        <div className="app__col app__col--center">
          <PriceGrid
            snapshot={snapshot}
            selected={{ city: selectedCity, product: selectedProduct }}
            onSelect={handleCellSelect}
          />
          {selectedPlayer != null ? (
            <PlayerDetail
              playerId={selectedPlayer}
              snapshot={snapshot}
              history={history[selectedPlayer] ?? []}
              onClose={() => setSelectedPlayer(null)}
            />
          ) : (
            <MarketChart cell={selectedCell} points={points} />
          )}
        </div>

        {/* Sağ: emir kitabı + şehir özeti */}
        <div className="app__col app__col--right">
          <OrderBook
            snapshot={snapshot}
            city={selectedCity}
            product={selectedProduct}
          />
        </div>
      </main>
    </div>
  );
}
