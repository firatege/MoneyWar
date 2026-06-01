import { useCallback, useEffect, useRef, useState } from "react";
import type { ConnStatus, FeedItem, Snapshot } from "../types";

/** Feed tamponu üst sınırı (birikmiş olaylar). */
const FEED_CAP = 70;
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
}

/**
 * WS /ws bağlantısını yönetir: canlı snapshot, önceki snapshot (flash karşı-
 * laştırması için) ve tick'ler arası birikmiş olay feed'i. Kopmada üstel
 * backoff ile yeniden bağlanır.
 */
export function useGameSocket(): GameSocket {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [prev, setPrev] = useState<Snapshot | null>(null);
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const [status, setStatus] = useState<ConnStatus>("connecting");

  const wsRef = useRef<WebSocket | null>(null);
  const attemptRef = useRef(0);
  const lastFedTick = useRef<number>(-1);
  const closedByUs = useRef(false);

  const ingest = useCallback((snap: Snapshot) => {
    setSnapshot((current) => {
      setPrev(current);
      return snap;
    });
    // Olayları tick bazında biriktir; aynı tick iki kez gelirse atla.
    if (snap.tick !== lastFedTick.current) {
      lastFedTick.current = snap.tick;
      if (snap.recent_events.length > 0) {
        setFeed((old) => {
          const fresh: FeedItem[] = snap.recent_events.map((e, i) => ({
            ...e,
            key: `${snap.season}-${snap.tick}-${i}`,
          }));
          return [...fresh.reverse(), ...old].slice(0, FEED_CAP);
        });
      }
    }
  }, []);

  const connect = useCallback(() => {
    setStatus(attemptRef.current === 0 ? "connecting" : "connecting");
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

  return { snapshot, prev, feed, status };
}
