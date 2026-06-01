import { useEffect, useRef, useState } from "react";
import type { PricePoint, Snapshot } from "../types";

interface SeriesCache {
  points: PricePoint[];
  city: string;
  product: string;
}

/**
 * Seçili (city, product) için fiyat zaman serisini yönetir.
 * - İlk mount'ta /api/series ile geçmiş çeker.
 * - Snapshot tick'leri canlı olarak ekler (duplicate tick atlanır).
 * - city/product değişince yeniden yükler.
 */
export function useSeriesCache(
  city: string,
  product: string,
  snapshot: Snapshot | null,
): PricePoint[] {
  const [cache, setCache] = useState<SeriesCache>({ points: [], city, product });
  const loadedKey = useRef<string>("");

  // city/product değişince geçmiş yeniden yükle.
  useEffect(() => {
    const key = `${city}/${product}`;
    if (loadedKey.current === key) return;
    loadedKey.current = key;
    setCache({ points: [], city, product });

    fetch(`/api/series?city=${city}&product=${product}`)
      .then((r) => r.json())
      .then((data) => {
        const pts = (data.points ?? []) as PricePoint[];
        setCache({ points: pts, city, product });
      })
      .catch(() => {});
  }, [city, product]);

  // Canlı tick ekle.
  useEffect(() => {
    if (!snapshot) return;
    const cell = snapshot.prices.find((p) => p.city === city && p.product === product);
    if (!cell || cell.last_lira == null) return;
    const newPt: PricePoint = { tick: snapshot.tick, lira: cell.last_lira };
    setCache((prev) => {
      if (prev.city !== city || prev.product !== product) return prev;
      const last = prev.points.at(-1);
      if (last && last.tick >= newPt.tick) return prev;
      return { ...prev, points: [...prev.points, newPt] };
    });
  }, [snapshot, city, product]);

  return cache.city === city && cache.product === product ? cache.points : [];
}
