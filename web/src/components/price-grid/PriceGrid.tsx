import type { CSSProperties } from "react";
import type { PriceCell, Snapshot } from "../../types";
import type { BucketHistory } from "../../hooks/useGameSocket";
import { lira2 } from "../../lib/format";
import { CITY_SLUGS, PRODUCT_SLUGS } from "../../lib/catalog";
import { Sparkline } from "../sparkline/Sparkline";
import "./price-grid.css";

const CITIES = CITY_SLUGS;
const ALL_PRODUCTS = PRODUCT_SLUGS;

interface Props {
  snapshot: Snapshot | null;
  bucketHistory: BucketHistory;
  selected: { city: string; product: string };
  onSelect: (city: string, product: string) => void;
}

export function PriceGrid({ snapshot, bucketHistory, selected, onSelect }: Props) {
  const cellMap = new Map<string, PriceCell>();
  for (const c of snapshot?.prices ?? []) {
    cellMap.set(`${c.city}/${c.product}`, c);
  }

  return (
    <section className="pg panel">
      <div className="panel__head">
        <h2 className="panel__title">FİYAT IZGARASI</h2>
        <span className="panel__sub">
          {CITIES.length} şehir · {ALL_PRODUCTS.length} ürün
        </span>
      </div>

      <div
        className="pg__table"
        style={{ "--pg-cols": ALL_PRODUCTS.length } as CSSProperties}
      >
        <div className="pg__corner" />
        {ALL_PRODUCTS.map((p) => {
          const sample = cellMap.get(`istanbul/${p}`);
          return (
            <div key={p} className="pg__col-head">
              {sample?.product_label ?? p}
            </div>
          );
        })}

        {CITIES.map((city) => {
          const sample = cellMap.get(`${city}/pamuk`);
          return [
            <div key={`h-${city}`} className="pg__row-head">
              {sample?.city_label ?? city}
            </div>,
            ...ALL_PRODUCTS.map((product) => {
              const key = `${city}/${product}`;
              const c = cellMap.get(key);
              const active = selected.city === city && selected.product === product;
              return (
                <GridCell
                  key={key}
                  cell={c ?? null}
                  hist={bucketHistory[key] ?? []}
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
  hist,
  active,
  onSelect,
}: {
  cell: PriceCell | null;
  hist: number[];
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
  const dir = pct == null ? "flat" : pct > 0.5 ? "up" : pct < -0.5 ? "down" : "flat";

  // Yüzde metni hücreye sığmalı. Ölçümde 60 hücrede fiyat ile yüzde üst üste
  // biniyordu: "+246.8%" gibi değerler sütun genişliğini aşıyor. Büyük
  // sapmalarda ondalık bilgi taşımıyor (%246,8 ile %247 arasında izleyici
  // için fark yok), o yüzden basamak sayısı büyüklüğe göre kısalıyor.
  const pctText =
    pct == null
      ? "—"
      : Math.abs(pct) >= 1000
        ? (pct > 0 ? "+" : "−") + "999%"
        : (pct > 0 ? "+" : "") + pct.toFixed(Math.abs(pct) >= 100 ? 0 : 1) + "%";

  return (
    <button
      className={`pg__cell pg__cell--${dir}${active ? " pg__cell--active" : ""}`}
      onClick={onSelect}
      title={`${cell.city_label} / ${cell.product_label}`}
    >
      <span className="pg__top">
        <span className="pg__price num">{lira2(cell.last_lira)}</span>
        <span className="pg__pct">{pctText}</span>
      </span>
      <span className="pg__spark">
        <Sparkline values={hist} baseline={cell.baseline_lira} />
      </span>
    </button>
  );
}
