import type { PriceCell, Snapshot } from "../../types";
import { lira2 } from "../../lib/format";
import "./price-grid.css";

const CITIES = ["istanbul", "ankara", "izmir", "bursa", "konya"];
const PRODUCTS_RAW = ["pamuk", "bugday", "zeytin"];
const PRODUCTS_FIN = ["kumas", "un", "zeytinyagi"];
const ALL_PRODUCTS = [...PRODUCTS_RAW, ...PRODUCTS_FIN];

interface Props {
  snapshot: Snapshot | null;
  selected: { city: string; product: string };
  onSelect: (city: string, product: string) => void;
}

export function PriceGrid({ snapshot, selected, onSelect }: Props) {
  const cellMap = new Map<string, PriceCell>();
  for (const c of snapshot?.prices ?? []) {
    cellMap.set(`${c.city}/${c.product}`, c);
  }

  return (
    <section className="pg panel">
      <div className="panel__head">
        <h2 className="panel__title">FİYAT IZGARASI</h2>
        <span className="panel__sub">5 şehir · 6 ürün</span>
      </div>

      <div className="pg__table">
        {/* Header satırı: ürünler */}
        <div className="pg__corner" />
        {ALL_PRODUCTS.map((p) => {
          const sample = cellMap.get(`istanbul/${p}`);
          return (
            <div key={p} className="pg__col-head">
              {sample?.product_label ?? p}
            </div>
          );
        })}

        {/* Şehir satırları */}
        {CITIES.map((city) => {
          const sample = cellMap.get(`${city}/pamuk`);
          return [
            <div key={`h-${city}`} className="pg__row-head">
              {sample?.city_label ?? city}
            </div>,
            ...ALL_PRODUCTS.map((product) => {
              const c = cellMap.get(`${city}/${product}`);
              const active =
                selected.city === city && selected.product === product;
              return (
                <GridCell
                  key={`${city}/${product}`}
                  cell={c ?? null}
                  active={active}
                  onSelect={() => onSelect(city, product)}
                />
              );
            }),
          ];
        })}
      </div>
    </section>
  );
}

function GridCell({
  cell,
  active,
  onSelect,
}: {
  cell: PriceCell | null;
  active: boolean;
  onSelect: () => void;
}) {
  if (!cell) {
    return <div className="pg__cell pg__cell--empty" />;
  }

  const pct =
    cell.last_lira != null && cell.baseline_lira > 0
      ? ((cell.last_lira - cell.baseline_lira) / cell.baseline_lira) * 100
      : null;
  const dir =
    pct == null ? "flat" : pct > 0.5 ? "up" : pct < -0.5 ? "down" : "flat";

  return (
    <button
      className={`pg__cell pg__cell--${dir}${active ? " pg__cell--active" : ""}`}
      onClick={onSelect}
      title={`${cell.city_label} / ${cell.product_label}`}
    >
      <span className="pg__price num">{lira2(cell.last_lira)}</span>
      <span className="pg__pct">
        {pct == null
          ? "—"
          : (pct > 0 ? "+" : "") + pct.toFixed(1) + "%"}
      </span>
      <MiniBar buyQty={cell.buy_qty} sellQty={cell.sell_qty} />
    </button>
  );
}

/** Alış/satış hacmini gösteren ufak bar çifti. */
function MiniBar({ buyQty, sellQty }: { buyQty: number; sellQty: number }) {
  const total = buyQty + sellQty;
  if (total === 0) return <span className="pg__minibar" />;
  const buyPct = Math.round((buyQty / total) * 100);
  return (
    <span className="pg__minibar" title={`AL ${buyQty} · SAT ${sellQty}`}>
      <span
        className="pg__minibar-buy"
        style={{ width: `${buyPct}%` }}
      />
    </span>
  );
}
