// Snapshot akışından türetilen istemci-tarafı durumun saf (yan etkisiz)
// hesaplamaları. useGameSocket bunları çağırır; testler de doğrudan bunları
// hedefler — React'e ya da WS'e bağımlılık yok.

import type { FeedItem, Snapshot } from "../types";
import { isStoryKind } from "../types";

/** Feed tamponu üst sınırı (birikmiş olaylar). */
export const FEED_CAP = 60;
/**
 * Entrika olayları için ayrı kota. Sıradan eşleşmeler dakikada onlarca
 * satır üretiyor ve tek kotalı listede nadir olan tekel/savaş/iflas
 * satırlarını birkaç tick içinde dışarı itiyordu — izleyicinin asıl takip
 * ettiği şey kayboluyordu.
 */
export const STORY_FEED_CAP = 40;
/** Oyuncu başına PnL geçmişi üst sınırı (tick). Sezon uzunluğuyla eşleşir. */
export const HISTORY_CAP = 350;
/** Bucket başına sparkline geçmişi üst sınırı (tick). */
// Sunucudan gelen tohum 60 noktaya kadar olabiliyor (bkz. `build_all_history`);
// üstüne canlı tick'ler eklenecek. Eski değer 26'ydı ve geçmiş yalnız
// tarayıcıda biriktiği için grafik sayfayı açtığın tick'ten başlıyordu.
export const BUCKET_HIST_CAP = 90;
/** Genel piyasa serisi üst sınırı (tick). Sezon uzunluğuyla eşleşir. */
export const MARKET_CAP = 350;

export interface PnlPoint {
  tick: number;
  pnl: number;
}

/** Oyuncu id → bu sezondaki PnL zaman serisi. */
export type PlayerHistory = Record<number, PnlPoint[]>;

/** Bucket (`city/product`) → kısa fiyat geçmişi (sparkline için). */
export type BucketHistory = Record<string, number[]>;

/** Tek tick için genel piyasa metrikleri. */
export interface MarketPoint {
  tick: number;
  /** Endeks: tüm bucket'larda ort(last/baseline)×100. 100 = baz. */
  index: number;
  /** O tick eşleşen toplam birim (işlem hacmi). */
  volume: number;
  /** Ham madde endeksi (sadece ham bucket'lar). */
  rawIndex: number;
  /** Mamul endeksi (sadece mamul bucket'lar). */
  finIndex: number;
}

/**
 * Yeni snapshot'ın olaylarını mevcut feed'in başına ekler (en yeni üstte),
 * `FEED_CAP` ile kırpar. Olay yoksa feed'i olduğu gibi döndürür.
 */
export function mergeFeed(old: FeedItem[], snap: Snapshot): FeedItem[] {
  if (snap.recent_events.length === 0) return old;
  const fresh: FeedItem[] = snap.recent_events.map((e, i) => ({
    ...e,
    key: `${snap.season}-${snap.tick}-${i}`,
  }));
  const merged = [...fresh.reverse(), ...old];
  // İki ayrı kota: sıradan olaylar entrikayı listeden atamaz.
  const keep = new Set<string>();
  let plain = 0;
  let story = 0;
  for (const item of merged) {
    if (isStoryKind(item.kind)) {
      if (story++ < STORY_FEED_CAP) keep.add(item.key);
    } else if (plain++ < FEED_CAP) {
      keep.add(item.key);
    }
  }
  return merged.filter((item) => keep.has(item.key));
}

/**
 * Her oyuncunun PnL serisine bu tick'in noktasını ekler. Aynı tick yeniden
 * gelirse (duplicate) seriyi büyütmez. `HISTORY_CAP` ile kırpar.
 */
export function appendHistory(old: PlayerHistory, snap: Snapshot): PlayerHistory {
  const next: PlayerHistory = { ...old };
  for (const p of snap.leaderboard) {
    const series = next[p.id] ?? [];
    const last = series.at(-1);
    if (!last || last.tick < snap.tick) {
      next[p.id] = [...series, { tick: snap.tick, pnl: p.pnl_lira }].slice(
        -HISTORY_CAP,
      );
    }
  }
  return next;
}

/**
 * Her bucket'ın (`city/product`) son fiyatını sparkline geçmişine ekler.
 * `last_lira` boşsa o bucket atlanır. `BUCKET_HIST_CAP` ile kırpar.
 */
export function appendBucketHistory(
  old: BucketHistory,
  snap: Snapshot,
): BucketHistory {
  const next: BucketHistory = { ...old };
  for (const c of snap.prices) {
    if (c.last_lira == null) continue;
    const key = `${c.city}/${c.product}`;
    next[key] = [...(next[key] ?? []), c.last_lira].slice(-BUCKET_HIST_CAP);
  }
  return next;
}

/**
 * Bu tick'in genel piyasa metriklerini hesaplar: tüm/ham/mamul fiyat
 * endeksleri (last/baseline×100 ortalaması) ve eşleşen toplam hacim.
 * Geçerli bucket yoksa endeksler 100 (baz) döner.
 */
export function computeMarketPoint(snap: Snapshot): MarketPoint {
  let sum = 0;
  let n = 0;
  let rawSum = 0;
  let rawN = 0;
  let finSum = 0;
  let finN = 0;
  for (const c of snap.prices) {
    if (c.last_lira == null || c.baseline_lira <= 0) continue;
    const ratio = (c.last_lira / c.baseline_lira) * 100;
    sum += ratio;
    n += 1;
    if (c.is_raw) {
      rawSum += ratio;
      rawN += 1;
    } else {
      finSum += ratio;
      finN += 1;
    }
  }
  const volume = snap.recent_events
    .filter((e) => e.kind === "match")
    .reduce((s, e) => s + (e.qty ?? 0), 0);
  return {
    tick: snap.tick,
    index: n > 0 ? sum / n : 100,
    volume,
    rawIndex: rawN > 0 ? rawSum / rawN : 100,
    finIndex: finN > 0 ? finSum / finN : 100,
  };
}

/** Yeni piyasa noktasını seriye ekler, `MARKET_CAP` ile kırpar. */
export function appendMarket(old: MarketPoint[], point: MarketPoint): MarketPoint[] {
  return [...old, point].slice(-MARKET_CAP);
}

// ─── Kişi bazlı işlem istatistiği ──────────────────────────────────────────

/** Ürün bazlı alım/satım özeti. */
export interface ProductStat {
  product: string;
  buy_qty: number;
  sell_qty: number;
}

/** Oyuncu id → ürün bazlı kümülatif işlem istatistiği (sezon). */
export type PlayerTradeStats = Record<number, Record<string, { buy: number; sell: number }>>;

/**
 * Bu tick'teki match olaylarından kişi bazlı ürün istatistiğini günceller.
 * Sezon sıfırlandığında `{}` geçilir.
 */
export function appendTradeStats(
  old: PlayerTradeStats,
  snap: { recent_events: import("../types").EventDto[]; tick: number },
): PlayerTradeStats {
  const events = snap.recent_events.filter((e) => e.kind === "match");
  if (events.length === 0) return old;

  const next: PlayerTradeStats = { ...old };

  for (const e of events) {
    if (!e.product) continue;
    const product = e.product;
    const qty = e.qty ?? 0;

    if (e.buyer_id != null) {
      if (!next[e.buyer_id]) next[e.buyer_id] = {};
      const b = next[e.buyer_id][product] ?? { buy: 0, sell: 0 };
      next[e.buyer_id] = { ...next[e.buyer_id], [product]: { ...b, buy: b.buy + qty } };
    }
    if (e.seller_id != null) {
      if (!next[e.seller_id]) next[e.seller_id] = {};
      const s = next[e.seller_id][product] ?? { buy: 0, sell: 0 };
      next[e.seller_id] = { ...next[e.seller_id], [product]: { ...s, sell: s.sell + qty } };
    }
  }

  return next;
}

/** Oyuncunun en çok işlem yaptığı ürünleri (azalan toplam hacme göre) döner. */
export function topProducts(stats: PlayerTradeStats, playerId: number): ProductStat[] {
  const byProduct = stats[playerId];
  if (!byProduct) return [];
  return Object.entries(byProduct)
    .map(([product, { buy, sell }]) => ({ product, buy_qty: buy, sell_qty: sell }))
    .sort((a, b) => (b.buy_qty + b.sell_qty) - (a.buy_qty + a.sell_qty))
    .slice(0, 5);
}
