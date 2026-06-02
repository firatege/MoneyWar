import type { Snapshot } from "../types";
import { signedCompact } from "./format";

export type NewsTone = "up" | "down" | "flat" | "accent";

export interface NewsItem {
  id: string;
  label: string; // ana metin
  value?: string; // sağdaki vurgulu değer (örn. +%16, +191B)
  tone: NewsTone;
}

const SHOWN_KINDS = new Set(["Sanayici", "Tüccar"]);

/**
 * Snapshot + önceki snapshot'tan haber manşetleri üretir:
 * lider, en çok yükselen/düşen ürün fiyatları, sırada yükselen oyuncular.
 */
export function buildNews(snap: Snapshot | null, prev: Snapshot | null): NewsItem[] {
  if (!snap) return [];
  const items: NewsItem[] = [];

  // 1. Lider (Sanayici + Tüccar)
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

  // 2. Fiyat hareketleri — baseline'a göre yüzde değişim
  const moves = snap.prices
    .filter((c) => c.last_lira != null && c.baseline_lira > 0)
    .map((c) => ({
      label: `${c.product_label} ${c.city_label}`,
      pct: ((c.last_lira! - c.baseline_lira) / c.baseline_lira) * 100,
    }))
    .sort((a, b) => b.pct - a.pct);

  const gainers = moves.slice(0, 4);
  const losers = moves.slice(-4).reverse();

  for (const g of gainers) {
    if (g.pct <= 0.5) continue;
    items.push({
      id: `g-${g.label}`,
      label: g.label,
      value: `▲%${g.pct.toFixed(1)}`,
      tone: "up",
    });
  }
  for (const l of losers) {
    if (l.pct >= -0.5) continue;
    items.push({
      id: `l-${l.label}`,
      label: l.label,
      value: `▼%${Math.abs(l.pct).toFixed(1)}`,
      tone: "down",
    });
  }

  // 3. Sırada yükselen oyuncular (önceki snapshot'a göre)
  if (prev) {
    const prevRank = new Map<number, number>();
    prev.leaderboard
      .filter((p) => p.npc_kind != null && SHOWN_KINDS.has(p.npc_kind))
      .forEach((p, i) => prevRank.set(p.id, i));
    ranked.forEach((p, i) => {
      const pr = prevRank.get(p.id);
      if (pr !== undefined && pr - i >= 2) {
        items.push({
          id: `climb-${p.id}`,
          label: `${p.name} yükseliyor`,
          value: `▲${pr - i} sıra`,
          tone: "up",
        });
      }
    });
  }

  return items;
}
