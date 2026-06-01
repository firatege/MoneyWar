import { useGameSocket } from "./hooks/useGameSocket";
import { SeasonHeader } from "./components/season-header/SeasonHeader";
import { TickerTape } from "./components/ticker-tape/TickerTape";
import { Leaderboard } from "./components/leaderboard/Leaderboard";
import { EventFeed } from "./components/event-feed/EventFeed";
import "./app.css";

export default function App() {
  const { snapshot, prev, feed, status } = useGameSocket();

  return (
    <div className="app">
      <SeasonHeader snapshot={snapshot} status={status} />
      <TickerTape prices={snapshot?.prices ?? []} />
      <main className="app__grid">
        <Leaderboard snapshot={snapshot} prev={prev} />
        <EventFeed feed={feed} />
      </main>
    </div>
  );
}
