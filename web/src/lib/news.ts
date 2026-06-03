import type { Snapshot } from "../types";
import { signedCompact } from "./format";

export type NewsTone = "up" | "down" | "flat" | "accent" | "warn";

export interface NewsItem {
  id: string;
  label: string;
  value?: string;
  tone: NewsTone;
}

const SHOWN_KINDS = new Set(["Sanayici", "Tüccar"]);

/** Kaç tick'te bir ticker güncellenir. */
export const TICKER_UPDATE_EVERY = 10;

/**
 * Bir güncelleme döngüsünde ticker'a eklenecek statik haberleri üretir.
 * Sadece `TICKER_UPDATE_EVERY` tick'te bir çağrılmalı.
 *
 * İçerik:
 *  1. Lider firma + PnL
 *  2. Feed'den gelen olay haberleri (harvest/news/factory_built)
 *  3. En yüksek fiyat hareketleri (baseline'a göre)
 */
export function buildStableNews(snap: Snapshot): NewsItem[] {
  const items: NewsItem[] = [];

  // 1. Lider
  const ranked = snap.leaderboard.filter(
    (p) => p.npc_kind != null && SHOWN_KINDS.has(p.npc_kind),
  );
  if (ranked[0]) {
    items.push({
      id: "leader",
      label: `LİDER  ${ranked[0].name}`,
      value: `${signedCompact(ranked[0].pnl_lira)}₺`,
      tone: "accent",
    });
  }

  // 2. Statik olaylar — feed'den hasat/olay/fabrika/kervan haberleri
  const EVENT_LABELS: Record<string, { prefix: string; tone: NewsTone }> = {
    harvest:        { prefix: "HASAT",   tone: "up" },
    news:           { prefix: "OLAY",    tone: "warn" },
    factory_built:  { prefix: "FABRİKA", tone: "accent" },
    factory_upgraded: { prefix: "YÜKSELT", tone: "accent" },
    factory_demolished: { prefix: "KAPATTI", tone: "down" },
    caravan:        { prefix: "KERVAN",  tone: "flat" },
    loan:           { prefix: "KREDİ",   tone: "flat" },
  };

  // Son tick'in anlamlı eventlerini al (summary zaten kısa + okunabilir)
  const seen = new Set<string>();
  for (const e of snap.recent_events) {
    const cfg = EVENT_LABELS[e.kind];
    if (!cfg) continue;
    const key = `${e.kind}-${e.summary.slice(0, 30)}`;
    if (seen.has(key)) continue;
    seen.add(key);
    items.push({
      id: `ev-${e.tick}-${e.kind}`,
      label: `${cfg.prefix}  ${e.summary}`,
      tone: cfg.tone,
    });
    if (items.length >= 12) break; // çok dolmasın
  }

  // 3. Fiyat hareketleri (baseline'dan %5+ sapanlar)
  const moves = snap.prices
    .filter((c) => c.last_lira != null && c.baseline_lira > 0)
    .map((c) => ({
      label: `${c.product_label} · ${c.city_label}`,
      pct: ((c.last_lira! - c.baseline_lira) / c.baseline_lira) * 100,
    }))
    .filter((m) => Math.abs(m.pct) >= 5)
    .sort((a, b) => Math.abs(b.pct) - Math.abs(a.pct))
    .slice(0, 4);

  for (const m of moves) {
    items.push({
      id: `price-${m.label}`,
      label: m.label,
      value: m.pct > 0 ? `▲%${m.pct.toFixed(0)}` : `▼%${Math.abs(m.pct).toFixed(0)}`,
      tone: m.pct > 0 ? "up" : "down",
    });
  }

  return items;
}
