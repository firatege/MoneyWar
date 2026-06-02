import type { FeedItem } from "../../types";
import { tickLabel } from "../../lib/format";
import "./event-feed.css";

interface Props {
  feed: FeedItem[];
}

const KIND_LABEL: Record<string, string> = {
  match: "EŞLEŞME",
  factory_built: "FABRİKA",
  production: "ÜRETİM",
  caravan: "KERVAN",
  harvest: "HASAT",
  loan: "KREDİ",
  news: "OLAY",
  other: "·",
};

export function EventFeed({ feed }: Props) {
  return (
    <section className="feed panel">
      <div className="panel__head">
        <h2 className="panel__title">AKIŞ</h2>
        <span className="panel__sub">canlı olaylar</span>
      </div>
      <ul className="feed__list">
        {feed.map((e) => (
          <li key={e.key} className={`feed__row feed__row--${e.kind}`}>
            <div className="feed__meta">
              <span className="feed__kind">{KIND_LABEL[e.kind] ?? e.kind}</span>
              <span className="feed__tick num">{tickLabel(e.tick)}</span>
            </div>
            <div className="feed__summary">{e.summary}</div>
          </li>
        ))}
        {feed.length === 0 && <li className="feed__empty">olay bekleniyor…</li>}
      </ul>
    </section>
  );
}
