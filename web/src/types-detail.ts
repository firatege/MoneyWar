// Drill-down endpoint'lerinin (crates/moneywar-web/src/detail.rs) karşılığı.
//
// Snapshot'tan ayrı tutuluyor: bu veriler WS yayınında değil, sayfa
// tıklanınca `/api/city|firm|factory` üzerinden çekiliyor.

export interface ActorRef {
  id: number;
  name: string;
  role: string | null;
}

export interface TradeRowDto {
  tick: number;
  city: string;
  product: string;
  product_label: string;
  quantity: number;
  price_lira: number;
  value_lira: number;
  buyer: ActorRef;
  seller: ActorRef;
}

export interface PairFlowDto {
  buyer: ActorRef;
  seller: ActorRef;
  trades: number;
  units: number;
  value_lira: number;
}

export interface ProductVolumeDto {
  product: string;
  product_label: string;
  is_raw: boolean;
  units: number;
  value_lira: number;
  avg_price_lira: number;
}

export interface StockRowDto {
  city: string;
  product: string;
  product_label: string;
  units: number;
  value_lira: number;
}

// ── Şehir ───────────────────────────────────────────────────────────────────

export interface CityActorDto {
  actor: ActorRef;
  factories: number;
  farms: number;
  stock_units: number;
  stock_value_lira: number;
  bought_units: number;
  sold_units: number;
}

export interface CityProductDto {
  product: string;
  product_label: string;
  factories: number;
  idle: number;
  produced_units: number;
}

export interface ProductStockDto {
  product: string;
  product_label: string;
  is_raw: boolean;
  units: number;
  value_lira: number;
  holders: number;
}

export interface CityDetail {
  city: string;
  label: string;
  tick: number;
  window_from_tick: number | null;
  factory_count: number;
  idle_factory_count: number;
  farm_count: number;
  employees: number;
  required_employees: number;
  factory_gini: number;
  stock_gini: number;
  actors: CityActorDto[];
  production: CityProductDto[];
  stock: ProductStockDto[];
  volume: ProductVolumeDto[];
  top_pairs: PairFlowDto[];
  recent_trades: TradeRowDto[];
}

// ── Firma ───────────────────────────────────────────────────────────────────

export interface FirmFactoryDto {
  id: number;
  city: string;
  city_label: string;
  product: string;
  product_label: string;
  level: number;
  employees: number;
  required_employees: number;
  idle: boolean;
  pending_units: number;
  produced_units: number;
}

export interface FirmFarmDto {
  id: number;
  city: string;
  product: string;
  product_label: string;
  level: number;
  output_per_tick: number;
}

export interface PartnerDto {
  actor: ActorRef;
  trade_count: number;
  trust_score: number;
  bought_units: number;
  sold_units: number;
  value_lira: number;
}

export interface FirmProductFlowDto {
  product: string;
  product_label: string;
  bought_units: number;
  sold_units: number;
  buy_value_lira: number;
  sell_value_lira: number;
  avg_buy_lira: number | null;
  avg_sell_lira: number | null;
}

export interface FirmDetail {
  actor: ActorRef;
  tick: number;
  window_from_tick: number | null;
  cash_lira: number;
  stock_value_lira: number;
  pnl_lira: number;
  rank: number | null;
  factories: FirmFactoryDto[];
  farms: FirmFarmDto[];
  stock: StockRowDto[];
  recent_trades: TradeRowDto[];
  partners: PartnerDto[];
  flow: FirmProductFlowDto[];
}

// ── Fabrika ─────────────────────────────────────────────────────────────────

export interface BatchDto {
  started_tick: number;
  completion_tick: number;
  units: number;
  ticks_remaining: number;
}

export interface InputStatusDto {
  product: string;
  product_label: string;
  is_primary: boolean;
  required: number;
  min_required: number;
  available: number;
  batches_covered: number;
  /** Şu an batch başlatılamıyor. Tek başına arıza değil — `idle` ile okunur. */
  blocking: boolean;
  partial: boolean;
}

export interface ProductionPointDto {
  tick: number;
  units: number;
}

export interface FactoryDetail {
  id: number;
  owner: ActorRef;
  city: string;
  city_label: string;
  product: string;
  product_label: string;
  tick: number;
  window_from_tick: number | null;
  level: number;
  employees: number;
  required_employees: number;
  staffing: number;
  idle: boolean;
  ticks_since_production: number | null;
  pending_units: number;
  batches: BatchDto[];
  inputs: InputStatusDto[];
  output_stock: number;
  unit_cost_lira: number | null;
  market_price_lira: number;
  margin_pct: number | null;
  production_history: ProductionPointDto[];
  produced_units: number;
  recent_sales: TradeRowDto[];
}
