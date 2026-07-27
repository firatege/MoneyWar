import { useMemo, useState } from "react";
import type { FeedItem } from "../../types";
import { tickLabel } from "../../lib/format";
import { styleFor } from "../../lib/story";
import "./event-feed.css";

/**
 * Katmanlı akış — akışın tekrarlı okunması bir ayar sorunu değil, ölçülmüş
 * bir dağılım sorunuydu.
 *
 * Bir sezonda ~398 anlatı olayı geçiyor (dakikada 23) ve bunun %60'ı iki
 * türden: fiyat kırma (%37) ve kin (%23). Düz bir liste bu yüzden aynı iki
 * satırı sonsuza kadar tekrar ediyordu; nadir ve önemli olan (iflas, tekel,
 * kartel) arada kaybolup gidiyordu.
 *
 * Çözüm filtre değil katman:
 *   1. Dönüm noktaları — nadir, tam kart, kalıcı
 *   2. Çekişme — orta, tek satır
 *   3. Fon gürültüsü — tek tek gösterilmez, türüne göre sayılır
 *
 * Böylece sık olan bilgi kaybolmuyor (sayacı duruyor) ama nadir olanı
 * bastırmıyor.
 */

/** 1. katman: sezonun hikâyesini değiştiren olaylar. */
const LANDMARK = new Set([
  "monopoly_formed",
  "monopoly_broken",
  "bankrupt",
  "cartel",
  "cartel_betrayed",
  // Devralma sezonun hikâyesini değiştirir: bir firma tesisini kaybeder,
  // rakibi o pazarda yoğunlaşır.
  "acquisition",
  "price_war_won",
]);

/** 2. katman: süregelen çekişme. */
const CONFLICT = new Set(["price_war", "supply_choke", "undercut"]);

/** Fon gürültüsünde tek tek değil sayarak gösterilecek türler. */
const NOISE_LABEL: Record<string, string> = {
  grudge: "kin",
  undercut: "fiyat kırma",
  match: "eşleşme",
  production: "üretim",
  factory_built: "yeni fabrika",
  factory_upgraded: "fabrika büyüdü",
  private_farm: "çiftlik",
  caravan: "kervan",
  harvest: "hasat",
  loan: "kredi",
  expired: "süre doldu",
};

/** Fon sayacının kapsadığı tick penceresi. */
const NOISE_WINDOW = 12;

interface Props {
  feed: FeedItem[];
  tick: number;
  onSelectFirm?: (id: number) => void;
}

export function LayeredFeed({ feed, tick, onSelectFirm }: Props) {
  const [showAll, setShowAll] = useState(false);

  const landmarks = useMemo(() => feed.filter((e) => LANDMARK.has(e.kind)).slice(0, 6), [feed]);
  const conflicts = useMemo(() => feed.filter((e) => CONFLICT.has(e.kind)).slice(0, 14), [feed]);

  /** Son pencerede tür başına kaç olay geçti. */
  const noise = useMemo(() => {
    const since = tick - NOISE_WINDOW;
    const counts = new Map<string, number>();
    for (const e of feed) {
      if (e.tick < since) continue;
      if (LANDMARK.has(e.kind)) continue;
      const label = NOISE_LABEL[e.kind];
      if (!label) continue;
      counts.set(label, (counts.get(label) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1]);
  }, [feed, tick]);

  const firmOf = (e: FeedItem) => e.seller_id ?? e.buyer_id ?? null;

  return (
    <section className="feed panel" aria-labelledby="feed-title">
      <div className="panel__head">
        <h2 id="feed-title" className="panel__title">
          AKIŞ
        </h2>
        <button
          type="button"
          className={`feed__toggle${showAll ? " feed__toggle--on" : ""}`}
          onClick={() => setShowAll((v) => !v)}
          aria-pressed={showAll}
        >
          {showAll ? "katmanlı görünüm" : "ham akış"}
        </button>
      </div>

      {showAll ? (
        <ul className="feed__list">
          {feed.slice(0, 120).map((e) => {
            const style = styleFor(e.kind);
            return (
              <li key={e.key} className="feed__row" data-weight={style.weight}>
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
          {feed.length === 0 && <li className="feed__empty">olay bekleniyor…</li>}
        </ul>
      ) : (
        <div className="feed__layers">
          {/* ── 1. Dönüm noktaları ─────────────────────────────────── */}
          <div className="feed__layer">
            <h3 className="feed__layer-title">Dönüm noktaları</h3>
            {landmarks.length === 0 ? (
              <p className="feed__layer-empty">
                henüz yok — tekel, iflas ve kartel buraya düşer
              </p>
            ) : (
              <ul className="feed__cards">
                {landmarks.map((e) => {
                  const style = styleFor(e.kind);
                  const firm = firmOf(e);
                  return (
                    <li key={e.key} className={`feed__card feed__card--${e.kind}`}>
                      <div className="feed__card-head">
                        <span className="feed__card-kind">
                          {style.icon && <span className="feed__icon">{style.icon}</span>}
                          {style.label}
                        </span>
                        <span className="feed__tick num">{tickLabel(e.tick)}</span>
                      </div>
                      <p className="feed__card-text">{e.summary}</p>
                      {firm != null && onSelectFirm && (
                        <button
                          type="button"
                          className="feed__card-link"
                          onClick={() => onSelectFirm(firm)}
                        >
                          firmayı aç →
                        </button>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          {/* ── 2. Çekişme ─────────────────────────────────────────── */}
          <div className="feed__layer feed__layer--grow">
            <h3 className="feed__layer-title">Çekişme</h3>
            {conflicts.length === 0 ? (
              <p className="feed__layer-empty">piyasa sakin</p>
            ) : (
              <ul className="feed__list feed__list--inner">
                {conflicts.map((e) => {
                  const style = styleFor(e.kind);
                  return (
                    <li key={e.key} className="feed__row" data-weight={style.weight}>
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
              </ul>
            )}
          </div>

          {/* ── 3. Fon gürültüsü ───────────────────────────────────── */}
          <div className="feed__layer feed__noise">
            <h3 className="feed__layer-title">
              Fon <span className="feed__layer-note">son {NOISE_WINDOW} tick</span>
            </h3>
            {noise.length === 0 ? (
              <p className="feed__layer-empty">—</p>
            ) : (
              <ul className="feed__counts">
                {noise.map(([label, n]) => (
                  <li key={label} className="feed__count">
                    <span className="feed__count-n">{n}</span>
                    <span className="feed__count-label">{label}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
