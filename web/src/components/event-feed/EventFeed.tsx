import { useState } from "react";
import type { FeedItem } from "../../types";
import { tickLabel } from "../../lib/format";
import { isStory, styleFor } from "../../lib/story";
import "./event-feed.css";

interface Props {
  feed: FeedItem[];
}

export function EventFeed({ feed }: Props) {
  // İzleyicinin varsayılan derdi entrika; mekanik gürültü isteğe bağlı.
  const [storyOnly, setStoryOnly] = useState(true);
  const storyCount = feed.filter((e) => isStory(e.kind)).length;
  const rows = storyOnly ? feed.filter((e) => isStory(e.kind)) : feed;

  return (
    <section className="feed panel">
      <div className="panel__head">
        <h2 className="panel__title">AKIŞ</h2>
        <button
          type="button"
          className={`feed__toggle${storyOnly ? " feed__toggle--on" : ""}`}
          onClick={() => setStoryOnly((v) => !v)}
          aria-pressed={storyOnly}
        >
          {storyOnly ? `sadece entrika · ${storyCount}` : "her şey"}
        </button>
      </div>
      <ul className="feed__list">
        {rows.map((e) => {
          const style = styleFor(e.kind);
          const story = isStory(e.kind);
          return (
            <li
              key={e.key}
              className={`feed__row feed__row--${e.kind}${story ? " feed__row--story" : ""}`}
              data-weight={style.weight}
            >
              <div className="feed__meta">
                <span className="feed__kind">
                  {style.icon && <span className="feed__icon">{style.icon}</span>}
                  {style.label}
                </span>
                <span className="feed__tick num">{tickLabel(e.tick)}</span>
              </div>
              <div className="feed__summary">{e.summary}</div>
            </li>
          );
        })}
        {rows.length === 0 && (
          <li className="feed__empty">
            {storyOnly ? "henüz entrika yok…" : "olay bekleniyor…"}
          </li>
        )}
      </ul>
    </section>
  );
}
