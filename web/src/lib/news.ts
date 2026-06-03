import type { Snapshot } from "../types";

export type NewsTone = "up" | "down" | "flat" | "accent" | "warn";

export interface NewsItem {
  id: string;
  label: string;
  value?: string;
  tone: NewsTone;
}

export const TICKER_UPDATE_EVERY = 10;

/**
 * Sadece olay-bazlı statik haberler — hasat, kuraklık, fabrika, kervan.
 * Lider/fiyat değişimi YOK (her tick değişip "dinamik" görünüyor).
 * TICKER_UPDATE_EVERY tick'te bir çağrılır.
 */
export function buildStableNews(snap: Snapshot): NewsItem[] {
  const items: NewsItem[] = [];

  const EVENT_CFG: Record<string, { prefix: string; tone: NewsTone }> = {
    harvest:            { prefix: "HASAT",    tone: "up"     },
    news:               { prefix: "HABER",    tone: "warn"   },
    factory_built:      { prefix: "YENİ FAB", tone: "accent" },
    factory_upgraded:   { prefix: "YÜKSELT",  tone: "accent" },
    factory_demolished: { prefix: "KAPATTI",  tone: "down"   },
    private_farm:       { prefix: "TARLA",    tone: "up"     },
    caravan:            { prefix: "KERVAN",   tone: "flat"   },
    loan:               { prefix: "KREDİ",    tone: "flat"   },
  };

  const seen = new Set<string>();
  for (const e of snap.recent_events) {
    const cfg = EVENT_CFG[e.kind];
    if (!cfg) continue;
    const dedup = `${e.kind}-${e.summary.slice(0, 25)}`;
    if (seen.has(dedup)) continue;
    seen.add(dedup);
    items.push({
      id: `${e.tick}-${e.kind}-${items.length}`,
      label: `${cfg.prefix}  ${e.summary}`,
      tone: cfg.tone,
    });
    if (items.length >= 10) break;
  }

  // Fiyat şokları (±%8 üstü) — anlık değil, snapshot baz alınır
  for (const c of snap.prices) {
    if (c.last_lira == null || c.baseline_lira <= 0) continue;
    const pct = (c.last_lira - c.baseline_lira) / c.baseline_lira * 100;
    if (Math.abs(pct) < 8) continue;
    items.push({
      id: `shock-${c.city}-${c.product}`,
      label: `${c.product_label} · ${c.city_label}`,
      value: pct > 0 ? `▲%${pct.toFixed(0)}` : `▼%${Math.abs(pct).toFixed(0)}`,
      tone: pct > 0 ? "up" : "down",
    });
    if (items.length >= 14) break;
  }

  return items;
}
