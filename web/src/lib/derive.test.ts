import { describe, expect, it } from "vitest";
import type { EventDto, PriceCell, Snapshot } from "../types";
import {
  BUCKET_HIST_CAP,
  FEED_CAP,
  HISTORY_CAP,
  MARKET_CAP,
  appendBucketHistory,
  appendHistory,
  appendMarket,
  computeMarketPoint,
  mergeFeed,
  type MarketPoint,
} from "./derive";

// --- Test yardımcıları --------------------------------------------------

function cell(over: Partial<PriceCell> = {}): PriceCell {
  return {
    city: "istanbul",
    city_label: "İstanbul",
    product: "pamuk",
    product_label: "Pamuk",
    is_raw: true,
    baseline_lira: 100,
    last_lira: 100,
    avg5_lira: null,
    bid_lira: null,
    ask_lira: null,
    buy_qty: 0,
    sell_qty: 0,
    ...over,
  };
}

function event(over: Partial<EventDto> = {}): EventDto {
  return {
    tick: 1,
    kind: "match",
    summary: "",
    city: null,
    product: null,
    qty: null,
    price_lira: null,
    buyer_id: null,
    seller_id: null,
    ...over,
  };
}

function snap(over: Partial<Snapshot> = {}): Snapshot {
  return {
    season: 1,
    tick: 1,
    season_ticks: 90,
    seconds_per_tick: 5,
    leaderboard: [],
    prices: [],
    factories: [],
    caravans: [],
    private_farms: [],
    relations: [],
    recent_events: [],
    ...over,
  };
}

// --- mergeFeed ----------------------------------------------------------

describe("mergeFeed", () => {
  it("returns the same feed unchanged when there are no events", () => {
    const old = [{ ...event(), key: "old" }];
    expect(mergeFeed(old, snap({ recent_events: [] }))).toBe(old);
  });

  it("puts newest events on top and preserves older items below", () => {
    const old = [{ ...event({ summary: "eski" }), key: "k-old" }];
    const result = mergeFeed(
      old,
      snap({
        season: 2,
        tick: 7,
        recent_events: [event({ summary: "a" }), event({ summary: "b" })],
      }),
    );
    // reverse → "b" en üstte, sonra "a", sonra eski.
    expect(result.map((f) => f.summary)).toEqual(["b", "a", "eski"]);
    expect(result[0].key).toBe("2-7-1");
    expect(result[1].key).toBe("2-7-0");
  });

  it("clamps the feed to FEED_CAP", () => {
    const events = Array.from({ length: FEED_CAP + 20 }, (_, i) =>
      event({ summary: String(i) }),
    );
    const result = mergeFeed([], snap({ recent_events: events }));
    expect(result).toHaveLength(FEED_CAP);
  });
});

// --- appendHistory ------------------------------------------------------

describe("appendHistory", () => {
  const player = (id: number, pnl: number) => ({
    id,
    name: `P${id}`,
    role: "tuccar",
    npc_kind: null,
    cash_lira: 0,
    pnl_lira: pnl,
    is_npc: false,
  });

  it("appends one PnL point per player for a new tick", () => {
    const result = appendHistory(
      {},
      snap({ tick: 3, leaderboard: [player(1, 500), player(2, -200)] }),
    );
    expect(result[1]).toEqual([{ tick: 3, pnl: 500 }]);
    expect(result[2]).toEqual([{ tick: 3, pnl: -200 }]);
  });

  it("does not grow the series when the same tick arrives twice", () => {
    const first = appendHistory({}, snap({ tick: 3, leaderboard: [player(1, 500)] }));
    const second = appendHistory(
      first,
      snap({ tick: 3, leaderboard: [player(1, 999)] }),
    );
    expect(second[1]).toEqual([{ tick: 3, pnl: 500 }]);
  });

  it("clamps each player series to HISTORY_CAP", () => {
    let hist = {};
    for (let t = 0; t < HISTORY_CAP + 30; t++) {
      hist = appendHistory(hist, snap({ tick: t, leaderboard: [player(1, t)] }));
    }
    expect((hist as Record<number, unknown[]>)[1]).toHaveLength(HISTORY_CAP);
  });
});

// --- appendBucketHistory ------------------------------------------------

describe("appendBucketHistory", () => {
  it("appends last price keyed by city/product", () => {
    const result = appendBucketHistory(
      {},
      snap({ prices: [cell({ city: "izmir", product: "kumas", last_lira: 42 })] }),
    );
    expect(result["izmir/kumas"]).toEqual([42]);
  });

  it("skips cells with a null last price", () => {
    const result = appendBucketHistory({}, snap({ prices: [cell({ last_lira: null })] }));
    expect(result["istanbul/pamuk"]).toBeUndefined();
  });

  it("clamps each bucket to BUCKET_HIST_CAP", () => {
    let hist = {};
    for (let t = 0; t < BUCKET_HIST_CAP + 10; t++) {
      hist = appendBucketHistory(hist, snap({ prices: [cell({ last_lira: t })] }));
    }
    expect((hist as Record<string, unknown[]>)["istanbul/pamuk"]).toHaveLength(
      BUCKET_HIST_CAP,
    );
  });
});

// --- computeMarketPoint -------------------------------------------------

describe("computeMarketPoint", () => {
  it("computes the index as the average of last/baseline×100", () => {
    const point = computeMarketPoint(
      snap({
        tick: 5,
        prices: [
          cell({ last_lira: 120, baseline_lira: 100, is_raw: true }),
          cell({ last_lira: 80, baseline_lira: 100, is_raw: false }),
        ],
      }),
    );
    expect(point.index).toBe(100); // (120 + 80) / 2
    expect(point.rawIndex).toBe(120);
    expect(point.finIndex).toBe(80);
    expect(point.tick).toBe(5);
  });

  it("falls back to baseline 100 when no valid bucket exists", () => {
    const point = computeMarketPoint(
      snap({ prices: [cell({ last_lira: null }), cell({ baseline_lira: 0 })] }),
    );
    expect(point.index).toBe(100);
    expect(point.rawIndex).toBe(100);
    expect(point.finIndex).toBe(100);
  });

  it("counts volume only from match events", () => {
    const point = computeMarketPoint(
      snap({
        recent_events: [
          event({ kind: "match", qty: 10 }),
          event({ kind: "match", qty: 5 }),
          event({ kind: "news", qty: 1000 }),
        ],
      }),
    );
    expect(point.volume).toBe(15);
  });
});

// --- appendMarket -------------------------------------------------------

describe("appendMarket", () => {
  const pt = (tick: number): MarketPoint => ({
    tick,
    index: 100,
    volume: 0,
    rawIndex: 100,
    finIndex: 100,
  });

  it("appends a point to the series", () => {
    expect(appendMarket([pt(1)], pt(2))).toEqual([pt(1), pt(2)]);
  });

  it("clamps the series to MARKET_CAP", () => {
    let series: MarketPoint[] = [];
    for (let t = 0; t < MARKET_CAP + 15; t++) series = appendMarket(series, pt(t));
    expect(series).toHaveLength(MARKET_CAP);
    expect(series[series.length - 1].tick).toBe(MARKET_CAP + 14);
  });
});
