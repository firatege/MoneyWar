// Backend DTO katmanının (crates/moneywar-web/src/dto.rs) birebir karşılığı.

export interface BrainTraitsDto {
  aggression: number;
  patience: number;
  risk: number;
  greed: number;
  pnl_trend: number;
}

export interface PlayerDto {
  id: number;
  name: string;
  role: string;
  npc_kind: string | null;
  cash_lira: number;
  pnl_lira: number;
  is_npc: boolean;
  /** "EXPAND" | "CORNER" | "PRICE_WAR" | "CONSOLIDATE" | "RETREAT" | null */
  goal: string | null;
  traits: BrainTraitsDto | null;
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
  city: string | null;
  product: string | null;
  qty: number | null;
  price_lira: number | null;
  buyer_id: number | null;
  seller_id: number | null;
}

export interface PrivateFarmDto {
  id: number;
  owner: number;
  city: string;
  product: string;
  level: number;
  output_per_tick: number;
}

export interface RelationDto {
  player_a: number;
  player_b: number;
  trade_count: number;
  total_units: number;
  trust_score: number;
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
  private_farms: PrivateFarmDto[];
  relations: RelationDto[];
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

export interface SeasonEntry {
  rank: number;
  id: number;
  name: string;
  npc_kind: string | null;
  pnl_lira: number;
  cash_lira: number;
}

export interface SeasonSummary {
  season: number;
  ticks_completed: number;
  top: SeasonEntry[];
}

export type ConnStatus = "connecting" | "open" | "closed";

/** Feed satırı — birikmiş, benzersiz anahtarlı. */
export interface FeedItem extends EventDto {
  key: string;
}
