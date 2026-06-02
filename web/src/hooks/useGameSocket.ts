import { useCallback, useEffect, useRef, useState } from "react";
import type { ConnStatus, FeedItem, Snapshot } from "../types";

/** Feed tamponu üst sınırı (birikmiş olaylar). */
const FEED_CAP = 60;
/** Oyuncu başına PnL geçmişi üst sınırı (tick). */
const HISTORY_CAP = 90;
/** Bucket başına sparkline geçmişi üst sınırı (tick). */
const BUCKET_HIST_CAP = 26;
/** Genel piyasa serisi üst sınırı (tick). */
const MARKET_CAP = 90;
/** Reconnect backoff (ms): üstel, tavanlı. */
const BACKOFF_MS = [500, 1000, 2000, 4000, 6000];

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/ws`;
}

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

interface GameSocket {
  snapshot: Snapshot | null;
  prev: Snapshot | null;
  feed: FeedItem[];
  status: ConnStatus;
  history: PlayerHistory;
  bucketHistory: BucketHistory;
  market: MarketPoint[];
}

/**
 * WS /ws bağlantısını yönetir: canlı snapshot, önceki snapshot (flash + sıra
 * değişimi için), tick'ler arası birikmiş olay feed'i ve oyuncu başına PnL
 * geçmişi (sezon değişince sıfırlanır). Kopmada üstel backoff ile yeniden bağlanır.
 */
export function useGameSocket(): GameSocket {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [prev, setPrev] = useState<Snapshot | null>(null);
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const [history, setHistory] = useState<PlayerHistory>({});
  const [bucketHistory, setBucketHistory] = useState<BucketHistory>({});
  const [market, setMarket] = useState<MarketPoint[]>([]);
  const [status, setStatus] = useState<ConnStatus>("connecting");

  const wsRef = useRef<WebSocket | null>(null);
  const attemptRef = useRef(0);
  const lastFedTick = useRef<number>(-1);
  const lastSeason = useRef<number>(-1);
  const closedByUs = useRef(false);

  const ingest = useCallback((snap: Snapshot) => {
    setSnapshot((current) => {
      setPrev(current);
      return snap;
    });

    const seasonChanged = snap.season !== lastSeason.current;

    // Sezon değişince feed + geçmiş sıfırla.
    if (seasonChanged) {
      lastSeason.current = snap.season;
      setFeed([]);
      setHistory({});
      setBucketHistory({});
      setMarket([]);
      lastFedTick.current = -1;
    }

    if (snap.tick === lastFedTick.current) return;
    lastFedTick.current = snap.tick;

    // Feed
    if (snap.recent_events.length > 0) {
      setFeed((old) => {
        const fresh: FeedItem[] = snap.recent_events.map((e, i) => ({
          ...e,
          key: `${snap.season}-${snap.tick}-${i}`,
        }));
        return [...fresh.reverse(), ...old].slice(0, FEED_CAP);
      });
    }

    // Oyuncu PnL geçmişi
    setHistory((old) => {
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
    });

    // Bucket başına fiyat geçmişi (sparkline)
    setBucketHistory((old) => {
      const next: BucketHistory = { ...old };
      for (const c of snap.prices) {
        if (c.last_lira == null) continue;
        const key = `${c.city}/${c.product}`;
        next[key] = [...(next[key] ?? []), c.last_lira].slice(-BUCKET_HIST_CAP);
      }
      return next;
    });

    // Genel piyasa endeksi + hacim
    setMarket((old) => {
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
      const point: MarketPoint = {
        tick: snap.tick,
        index: n > 0 ? sum / n : 100,
        volume,
        rawIndex: rawN > 0 ? rawSum / rawN : 100,
        finIndex: finN > 0 ? finSum / finN : 100,
      };
      return [...old, point].slice(-MARKET_CAP);
    });
  }, []);

  const connect = useCallback(() => {
    setStatus("connecting");
    const ws = new WebSocket(wsUrl());
    wsRef.current = ws;

    ws.onopen = () => {
      attemptRef.current = 0;
      setStatus("open");
    };
    ws.onmessage = (ev) => {
      try {
        ingest(JSON.parse(ev.data as string) as Snapshot);
      } catch {
        /* bozuk frame — atla */
      }
    };
    ws.onclose = () => {
      setStatus("closed");
      if (closedByUs.current) return;
      const delay = BACKOFF_MS[Math.min(attemptRef.current, BACKOFF_MS.length - 1)];
      attemptRef.current += 1;
      window.setTimeout(connect, delay);
    };
    ws.onerror = () => {
      ws.close();
    };
  }, [ingest]);

  useEffect(() => {
    closedByUs.current = false;
    connect();
    return () => {
      closedByUs.current = true;
      wsRef.current?.close();
    };
  }, [connect]);

  return { snapshot, prev, feed, status, history, bucketHistory, market };
}
