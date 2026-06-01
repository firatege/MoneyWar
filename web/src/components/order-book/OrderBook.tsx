import type { FactoryDto, PriceCell, Snapshot } from "../../types";
import { lira2 } from "../../lib/format";
import "./order-book.css";

interface Props {
  snapshot: Snapshot | null;
  city: string;
  product: string;
}

export function OrderBook({ snapshot, city, product }: Props) {
  const cell = snapshot?.prices.find(
    (p) => p.city === city && p.product === product,
  ) ?? null;

  const factories = (snapshot?.factories ?? []).filter(
    (f) => f.city === city && f.product === product,
  );

  const cityFactories = (snapshot?.factories ?? []).filter((f) => f.city === city);
  const cityCaravans = (snapshot?.caravans ?? []).filter(
    (c) => c.current_city === city,
  );

  return (
    <section className="ob panel">
      <div className="panel__head">
        <h2 className="panel__title">BUCKET</h2>
        <span className="panel__sub">
          {cell ? `${cell.city_label} · ${cell.product_label}` : "—"}
        </span>
      </div>

      <div className="ob__body">
        <BidAskBlock cell={cell} />
        <Divider />
        <FactoryBlock factories={factories} />
        <Divider />
        <CityBlock
          cityLabel={cell?.city_label ?? city}
          factories={cityFactories}
          caravanCount={cityCaravans.length}
        />
      </div>
    </section>
  );
}

function BidAskBlock({ cell }: { cell: PriceCell | null }) {
  if (!cell) return <Placeholder text="bucket seçin" />;
  const spread =
    cell.bid_lira != null && cell.ask_lira != null
      ? (cell.ask_lira - cell.bid_lira).toFixed(2)
      : null;

  return (
    <div className="ob__section">
      <div className="ob__sec-title">EMİR KİTABI</div>
      <div className="ob__bk">
        <div className="ob__bk-side ob__bk-side--ask">
          <div className="ob__bk-label">ASK / SAT</div>
          <div className="ob__bk-price num">{lira2(cell.ask_lira)}</div>
          <div className="ob__bk-qty num">{cell.sell_qty} birim</div>
        </div>
        <div className="ob__bk-mid">
          <div className="ob__bk-spread-label">SPREAD</div>
          <div className="ob__bk-spread num">{spread ?? "—"}</div>
        </div>
        <div className="ob__bk-side ob__bk-side--bid">
          <div className="ob__bk-label">BID / AL</div>
          <div className="ob__bk-price num">{lira2(cell.bid_lira)}</div>
          <div className="ob__bk-qty num">{cell.buy_qty} birim</div>
        </div>
      </div>
    </div>
  );
}

function FactoryBlock({ factories }: { factories: FactoryDto[] }) {
  if (factories.length === 0) {
    return (
      <div className="ob__section">
        <div className="ob__sec-title">FABRİKALAR</div>
        <div className="ob__empty">bu bucket'ta fabrika yok</div>
      </div>
    );
  }
  return (
    <div className="ob__section">
      <div className="ob__sec-title">FABRİKALAR ({factories.length})</div>
      {factories.map((f) => (
        <div key={f.id} className={`ob__factory ${f.idle ? "ob__factory--idle" : ""}`}>
          <span className="ob__factory-id">#{f.id}</span>
          <span className="ob__factory-owner">sahibi {f.owner}</span>
          <span className="ob__factory-pending num">{f.pending_units} bk</span>
          {f.idle && <span className="ob__factory-idle-badge">ATIL</span>}
        </div>
      ))}
    </div>
  );
}

function CityBlock({
  cityLabel,
  factories,
  caravanCount,
}: {
  cityLabel: string;
  factories: FactoryDto[];
  caravanCount: number;
}) {
  const active = factories.filter((f) => !f.idle).length;
  const idle = factories.filter((f) => f.idle).length;
  return (
    <div className="ob__section">
      <div className="ob__sec-title">{cityLabel.toUpperCase()} ÖZET</div>
      <div className="ob__stat-row">
        <span className="ob__stat-k">FABRİKA AKTİF</span>
        <span className="ob__stat-v num">{active}</span>
      </div>
      <div className="ob__stat-row">
        <span className="ob__stat-k">FABRİKA ATIL</span>
        <span className={`ob__stat-v num ${idle > 0 ? "ob__stat-v--warn" : ""}`}>
          {idle}
        </span>
      </div>
      <div className="ob__stat-row">
        <span className="ob__stat-k">KERVAN (demirli)</span>
        <span className="ob__stat-v num">{caravanCount}</span>
      </div>
    </div>
  );
}

function Divider() {
  return <div className="ob__divider" />;
}

function Placeholder({ text }: { text: string }) {
  return (
    <div className="ob__section">
      <div className="ob__empty">{text}</div>
    </div>
  );
}
