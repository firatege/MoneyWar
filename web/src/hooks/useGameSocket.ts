import { useCallback, useEffect, useRef, useState } from "react";
import type { ConnStatus, FeedItem, SeasonSummary, Snapshot } from "../types";
import {
  appendBucketHistory,
  appendHistory,
  appendMarket,
  appendTradeStats,
  computeMarketPoint,
  mergeFeed,
} from "../lib/derive";
import type { BucketHistory, MarketPoint, PlayerHistory, PlayerTradeStats } from "../lib/derive";

// Türetme tipleri lib/derive'da yaşar; geri-uyumluluk için yeniden dışa aktar.
export type { BucketHistory, MarketPoint, PlayerHistory, PlayerTradeStats, PnlPoint } from "../lib/derive";

/** Reconnect backoff (ms): üstel, tavanlı. */
const BACKOFF_MS = [500, 1000, 2000, 4000, 6000];

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/ws`;
}

interface GameSocket {
  snapshot: Snapshot | null;
  prev: Snapshot | null;
  feed: FeedItem[];
  status: ConnStatus;
  history: PlayerHistory;
  bucketHistory: BucketHistory;
  market: MarketPoint[];
  tradeStats: PlayerTradeStats;
  /** Son sezon özetleri (en yeni önce). */
  seasons: SeasonSummary[];
  /** Mevcut sezonu sıfırla. */
  resetSeason: () => Promise<void>;
}

/**
 * WS /ws bağlantısını yönetir. Sezon değişince tüm geçmiş sıfırlanır.
 * Kopmada üstel backoff ile yeniden bağlanır.
 */
export function useGameSocket(): GameSocket {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [prev, setPrev] = useState<Snapshot | null>(null);
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const [history, setHistory] = useState<PlayerHistory>({});
  const [bucketHistory, setBucketHistory] = useState<BucketHistory>({});
  const [market, setMarket] = useState<MarketPoint[]>([]);
  const [tradeStats, setTradeStats] = useState<PlayerTradeStats>({});
  const [status, setStatus] = useState<ConnStatus>("connecting");
  const [seasons, setSeasons] = useState<SeasonSummary[]>([]);

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

    if (seasonChanged) {
      lastSeason.current = snap.season;
      setFeed([]);
      setHistory({});
      setBucketHistory({});
      setMarket([]);
      setTradeStats({});
      lastFedTick.current = -1;
    }

    if (snap.tick === lastFedTick.current) return;
    lastFedTick.current = snap.tick;

    setFeed((old) => mergeFeed(old, snap));
    setHistory((old) => appendHistory(old, snap));
    setBucketHistory((old) => appendBucketHistory(old, snap));
    setMarket((old) => appendMarket(old, computeMarketPoint(snap)));
    setTradeStats((old) => appendTradeStats(old, snap));

    // Ticker: her TICKER_UPDATE_EVERY tick'te bir güncelle.
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

  // Sezon geçmişini yükle (sayfa açılışında + her sezon değişiminde).
  const fetchSeasons = useCallback(async () => {
    try {
      const res = await fetch("/api/seasons");
      if (res.ok) setSeasons((await res.json()) as SeasonSummary[]);
    } catch { /* ağ hatası — sessiz */ }
  }, []);

  const resetSeason = useCallback(async () => {
    await fetch("/api/reset", { method: "POST" });
    await fetchSeasons();
  }, [fetchSeasons]);

  useEffect(() => {
    void fetchSeasons();
  }, [fetchSeasons]);

  // Her yeni sezon başlangıcında listeyi güncelle.
  const lastSeasonForFetch = useRef<number>(-1);
  useEffect(() => {
    if (snapshot && snapshot.season !== lastSeasonForFetch.current) {
      lastSeasonForFetch.current = snapshot.season;
      void fetchSeasons();
    }
  }, [snapshot?.season, fetchSeasons]);

  useEffect(() => {
    closedByUs.current = false;
    connect();
    return () => {
      closedByUs.current = true;
      wsRef.current?.close();
    };
  }, [connect]);

  return { snapshot, prev, feed, status, history, bucketHistory, market, tradeStats, seasons, resetSeason };
}
