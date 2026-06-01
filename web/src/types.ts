// Backend DTO katmanının (crates/moneywar-web/src/dto.rs) birebir karşılığı.

export interface PlayerDto {
  id: number;
  name: string;
  role: string;
  npc_kind: string | null;
  cash_lira: number;
  pnl_lira: number;
  is_npc: boolean;
}

export interface PriceCell {
  city: string;
  city_label: string;
  product: string;
  product_label: string;
  is_raw: boolean;
  baseline_lira: number;
  last_lira: number | null;
  avg5_lira: number | null;
  bid_lira: number | null;
  ask_lira: number | null;
  buy_qty: number;
  sell_qty: number;
}

export interface FactoryDto {
  id: number;
  owner: number;
  city: string;
  product: string;
  pending_units: number;
  idle: boolean;
}

export interface CaravanDto {
  id: number;
  owner: number;
  idle: boolean;
  current_city: string | null;
  cargo_units: number;
}

export interface EventDto {
  tick: number;
  kind: string;
  summary: string;
}

export interface Snapshot {
  season: number;
  tick: number;
  season_ticks: number;
  seconds_per_tick: number;
  leaderboard: PlayerDto[];
  prices: PriceCell[];
  factories: FactoryDto[];
  caravans: CaravanDto[];
  recent_events: EventDto[];
}

export interface PricePoint {
  tick: number;
  lira: number;
}

export interface PriceSeries {
  city: string;
  product: string;
  points: PricePoint[];
}

export type ConnStatus = "connecting" | "open" | "closed";

/** Feed satırı — birikmiş, benzersiz anahtarlı. */
export interface FeedItem extends EventDto {
  key: string;
}
