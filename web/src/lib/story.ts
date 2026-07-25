import { isStoryKind } from "../types";

/** Bir olay türünün ekrandaki kimliği: rozet metni, ikon, vurgu düzeyi. */
export interface StoryStyle {
  label: string;
  icon: string;
  /** Yüksek olanlar feed'de ve haritada daha güçlü vurgulanır. */
  weight: 1 | 2 | 3;
}

/**
 * Anlatı olayları için rozet/ikon sözlüğü. İzleyicinin akışa bakıp ne
 * olduğunu tek bakışta anlaması bu tablodan geçiyor — sıradan mekanik
 * olaylar (eşleşme, üretim) sönük, entrika olayları parlak.
 */
const STORY_STYLE: Record<string, StoryStyle> = {
  monopoly_formed: { label: "TEKEL", icon: "👑", weight: 3 },
  monopoly_broken: { label: "TEKEL KIRILDI", icon: "⚡", weight: 3 },
  price_war: { label: "SAVAŞ", icon: "⚔️", weight: 3 },
  price_war_won: { label: "ZAFER", icon: "🏳️", weight: 3 },
  bankrupt: { label: "İFLAS", icon: "💀", weight: 3 },
  cartel: { label: "KARTEL", icon: "🤝", weight: 3 },
  cartel_betrayed: { label: "İHANET", icon: "🗡️", weight: 3 },
  supply_choke: { label: "TEDARİK", icon: "🔒", weight: 2 },
  undercut: { label: "FİYAT KIRMA", icon: "✂️", weight: 2 },
  grudge: { label: "KİN", icon: "🔥", weight: 1 },
};

const PLAIN_STYLE: Record<string, StoryStyle> = {
  match: { label: "EŞLEŞME", icon: "", weight: 1 },
  factory_built: { label: "FABRİKA", icon: "", weight: 1 },
  factory_upgraded: { label: "FABRİKA", icon: "", weight: 1 },
  private_farm: { label: "ÇİFTLİK", icon: "", weight: 1 },
  production: { label: "ÜRETİM", icon: "", weight: 1 },
  caravan: { label: "KERVAN", icon: "", weight: 1 },
  harvest: { label: "HASAT", icon: "", weight: 1 },
  loan: { label: "KREDİ", icon: "", weight: 1 },
  news: { label: "OLAY", icon: "", weight: 1 },
  expired: { label: "SÜRE DOLDU", icon: "", weight: 1 },
};

const FALLBACK: StoryStyle = { label: "·", icon: "", weight: 1 };

export function styleFor(kind: string): StoryStyle {
  return STORY_STYLE[kind] ?? PLAIN_STYLE[kind] ?? FALLBACK;
}

/** Bu olay entrika akışına mı ait (izleyicinin asıl takip ettiği şey)? */
export function isStory(kind: string): boolean {
  return isStoryKind(kind);
}
