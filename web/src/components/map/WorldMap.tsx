import { useMemo } from "react";
import type { FeedItem, Snapshot } from "../../types";
import { isStory, styleFor } from "../../lib/story";
import {
  MAP_H,
  MAP_W,
  ROUTES,
  caravanPoint,
  cityAt,
  deriveCityStates,
  type CityState,
} from "./geo";
import "./world-map.css";

interface Props {
  snapshot: Snapshot | null;
  feed: FeedItem[];
  /** Seçili şehir — panel ve vurgu bununla senkron. */
  selectedCity: string;
  onSelectCity: (city: string) => void;
  onSelectFirm: (id: number) => void;
}

/** Son N tick içindeki olaylar haritada "taze" sayılır ve parlar. */
const FLASH_TICKS = 6;

export function WorldMap({
  snapshot,
  feed,
  selectedCity,
  onSelectCity,
  onSelectFirm,
}: Props) {
  const cities = useMemo(() => deriveCityStates(snapshot), [snapshot]);
  const tick = snapshot?.tick ?? 0;

  // Haritada patlayan taze olaylar: son birkaç tick'in entrikaları.
  const flashes = useMemo(
    () =>
      feed
        .filter((e) => isStory(e.kind) && e.city && tick - e.tick <= FLASH_TICKS)
        .slice(0, 8),
    [feed, tick],
  );
  const flashByCity = useMemo(() => {
    const m = new Map<string, FeedItem>();
    for (const f of flashes) if (f.city && !m.has(f.city)) m.set(f.city, f);
    return m;
  }, [flashes]);

  const caravans = (snapshot?.caravans ?? []).filter((c) => !c.idle);

  return (
    <section className="map panel">
      <div className="panel__head">
        <h2 className="panel__title">HARİTA</h2>
        <span className="panel__sub">
          {summarize(cities)}
        </span>
      </div>

      <div className="map__frame">
        <svg
          viewBox={`0 0 ${MAP_W} ${MAP_H}`}
          className="map__svg"
          preserveAspectRatio="xMidYMid meet"
          role="img"
          aria-label="Şehirler, firmalar ve aralarındaki çatışmalar"
        >
          {/* Rotalar — kervanların izlediği yollar */}
          <g className="map__routes">
            {ROUTES.map(([a, b]) => {
              const from = cityAt(a);
              const to = cityAt(b);
              if (!from || !to) return null;
              return (
                <line
                  key={`${a}-${b}`}
                  x1={from.x}
                  y1={from.y}
                  x2={to.x}
                  y2={to.y}
                  className="map__route"
                />
              );
            })}
          </g>

          {/* Yoldaki kervanlar */}
          <g className="map__caravans">
            {caravans.map((c) => {
              const pt = caravanPoint(c.from_city, c.to_city, c.progress);
              if (!pt) return null;
              return (
                <circle
                  key={c.id}
                  cx={pt.x}
                  cy={pt.y}
                  r={3 + Math.min(c.cargo_units / 40, 3)}
                  className="map__caravan"
                >
                  <title>
                    {`${c.cargo_units} birim · ${c.from_city} → ${c.to_city}`}
                  </title>
                </circle>
              );
            })}
          </g>

          {/* Şehirler */}
          <g className="map__cities">
            {cities.map((c) => (
              <CityMarker
                key={c.node.slug}
                city={c}
                selected={c.node.slug === selectedCity}
                flash={flashByCity.get(c.node.slug)}
                onSelect={() => onSelectCity(c.node.slug)}
              />
            ))}
          </g>
        </svg>
      </div>

      <CityPanel
        city={cities.find((c) => c.node.slug === selectedCity)}
        onSelectFirm={onSelectFirm}
      />
    </section>
  );
}

function summarize(cities: CityState[]): string {
  const mono = cities.reduce((n, c) => n + c.monopolyCount, 0);
  const wars = cities.reduce((n, c) => n + c.warCount, 0);
  const chokes = cities.reduce((n, c) => n + c.chokeCount, 0);
  const parts: string[] = [];
  if (mono) parts.push(`${mono} tekel`);
  if (wars) parts.push(`${wars} savaş`);
  if (chokes) parts.push(`${chokes} boğma`);
  return parts.length ? parts.join(" · ") : "sakin";
}

interface MarkerProps {
  city: CityState;
  selected: boolean;
  flash?: FeedItem;
  onSelect: () => void;
}

function CityMarker({ city, selected, flash, onSelect }: MarkerProps) {
  const { node, factoryCount, monopolyCount, warCount, chokeCount } = city;
  // Düğüm boyutu şehirdeki sınai varlığı gösterir.
  const r = 16 + Math.min(factoryCount * 2.2, 20);
  const state = warCount > 0 ? "war" : monopolyCount > 0 ? "monopoly" : "calm";

  return (
    <g
      className={`map__city map__city--${state}${selected ? " map__city--selected" : ""}`}
      transform={`translate(${node.x} ${node.y})`}
      onClick={onSelect}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      aria-label={`${node.label}: ${factoryCount} fabrika, ${monopolyCount} tekel, ${warCount} savaş`}
    >
      {warCount > 0 && <circle r={r + 12} className="map__pulse" />}
      <circle r={r} className="map__disc" />
      <circle r={r} className="map__ring" />

      {monopolyCount > 0 && (
        <text className="map__crown" y={-r - 8} textAnchor="middle">
          {"👑".repeat(Math.min(monopolyCount, 3))}
        </text>
      )}
      {chokeCount > 0 && (
        <text className="map__lock" x={r - 2} y={-r + 6} textAnchor="middle">
          🔒
        </text>
      )}

      <text className="map__label" y={r + 18} textAnchor="middle">
        {node.label}
      </text>
      <text className="map__count num" y={5} textAnchor="middle">
        {factoryCount}
      </text>

      {flash && (
        <g className="map__flash">
          <text y={r + 36} textAnchor="middle" className="map__flash-text">
            {styleFor(flash.kind).icon} {styleFor(flash.kind).label}
          </text>
        </g>
      )}
    </g>
  );
}

function CityPanel({
  city,
  onSelectFirm,
}: {
  city?: CityState;
  onSelectFirm: (id: number) => void;
}) {
  if (!city) return null;
  if (city.firms.length === 0) {
    return (
      <div className="map__panel map__panel--empty">
        {city.node.label}'da henüz kimse yok.
      </div>
    );
  }
  return (
    <div className="map__panel">
      <div className="map__panel-head">{city.node.label}</div>
      <ul className="map__firms">
        {city.firms.map((f) => (
          <li key={f.id}>
            <button
              type="button"
              className="map__firm"
              onClick={() => onSelectFirm(f.id)}
            >
              <span className="map__firm-name">
                {f.monopolies > 0 && <span aria-hidden>👑 </span>}
                {f.name}
              </span>
              <span className="map__firm-meta num">
                {f.factories > 0 && <span>{f.factories} fab</span>}
                {f.atWar && <span className="map__tag map__tag--war">savaşta</span>}
                {f.choked && <span className="map__tag map__tag--choke">aç</span>}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
