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
import "./app.css";

const DEFAULT_CITY = "istanbul";
const DEFAULT_PRODUCT = "pamuk";

export default function App() {
  const { snapshot, prev, feed, status } = useGameSocket();
  const [selectedCity, setSelectedCity] = useState(DEFAULT_CITY);
  const [selectedProduct, setSelectedProduct] = useState(DEFAULT_PRODUCT);

  const points = useSeriesCache(selectedCity, selectedProduct, snapshot);
  const selectedCell =
    snapshot?.prices.find(
      (p) => p.city === selectedCity && p.product === selectedProduct,
    ) ?? null;

  const handleSelect = (city: string, product: string) => {
    setSelectedCity(city);
    setSelectedProduct(product);
  };

  return (
    <div className="app">
      <SeasonHeader snapshot={snapshot} status={status} />
      <TickerTape prices={snapshot?.prices ?? []} />
      <main className="app__grid">
        {/* Sol sütun: sıralama + akış */}
        <div className="app__col app__col--left">
          <Leaderboard snapshot={snapshot} prev={prev} />
          <EventFeed feed={feed} />
        </div>

        {/* Orta: fiyat ızgarası + grafik */}
        <div className="app__col app__col--center">
          <PriceGrid
            snapshot={snapshot}
            selected={{ city: selectedCity, product: selectedProduct }}
            onSelect={handleSelect}
          />
          <MarketChart cell={selectedCell} points={points} />
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
