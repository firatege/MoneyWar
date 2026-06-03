import type { NewsItem } from "../../hooks/useGameSocket";
import "./ticker-tape.css";

interface Props {
  news: NewsItem[];
}

function Item({ item }: { item: NewsItem }) {
  return (
    <span className="tick-cell">
      <span className="tick-cell__label">{item.label}</span>
      {item.value && (
        <span className={`tick-cell__value tick-cell__value--${item.tone}`}>
          {item.value}
        </span>
      )}
    </span>
  );
}

export function TickerTape({ news }: Props) {
  if (news.length === 0) {
    return <div className="ticker ticker--empty">haber akışı bekleniyor…</div>;
  }
  const run = (
    <div className="ticker__run">
      {news.map((item) => (
        <Item key={item.id} item={item} />
      ))}
    </div>
  );
  return (
    <div className="ticker" aria-label="haber akışı">
      <span className="ticker__badge">HABER</span>
      <div className="ticker__marquee">
        {run}
        {run}
      </div>
    </div>
  );
}
