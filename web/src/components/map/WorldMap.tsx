import { useMemo } from "react";
import type { FeedItem, Snapshot } from "../../types";
import { isStory, styleFor } from "../../lib/story";
import {
  LAND_PATHS,
  MAP_H,
  MAP_W,
  ROUTES,
  SEA_LABELS,
  caravanPoint,
  cityAt,
  deriveCityStates,
  project,
  routePath,
  type CityNode,
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

  const flashByCity = useMemo(() => {
    const m = new Map<string, FeedItem>();
    for (const e of feed) {
      if (!isStory(e.kind) || !e.city) continue;
      if (tick - e.tick > FLASH_TICKS) continue;
      if (!m.has(e.city)) m.set(e.city, e);
    }
    return m;
  }, [feed, tick]);

  const caravans = (snapshot?.caravans ?? []).filter((c) => !c.idle);

  return (
    <section className="map panel">
      <div className="panel__head">
        <h2 className="panel__title">HARİTA</h2>
        <span className="panel__sub">{summarize(cities)}</span>
      </div>

      <div className="map__frame">
        <svg
          viewBox={`0 0 ${MAP_W} ${MAP_H}`}
          className="map__svg"
          preserveAspectRatio="xMidYMid meet"
          role="img"
          aria-label="Batı Anadolu haritası: şehirler, ticaret yolları ve firmalar arası çatışmalar"
        >
          <defs>
            {/* Kara yüzeyine hafif yükseklik hissi veren dikey geçiş. */}
            <linearGradient id="mw-land" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--map-land-hi)" />
              <stop offset="100%" stopColor="var(--map-land-lo)" />
            </linearGradient>
            {/* Kıyı boyunca sığ su bandı — deniz haritası ipucu. */}
            <filter id="mw-shelf" x="-20%" y="-20%" width="140%" height="140%">
              <feGaussianBlur stdDeviation="4" />
            </filter>
          </defs>

          <rect width={MAP_W} height={MAP_H} className="map__sea" />

          {/* Paralel/meridyen ağı — kartografik doku, çok soluk */}
          <g className="map__graticule">
            {[36, 37, 38, 39, 40, 41, 42].map((lat) => {
              const { y } = project(0, lat);
              return <line key={`lat${lat}`} x1={0} y1={y} x2={MAP_W} y2={y} />;
            })}
            {[26, 28, 30, 32, 34, 36, 38].map((lon) => {
              const { x } = project(lon, 0);
              return <line key={`lon${lon}`} x1={x} y1={0} x2={x} y2={MAP_H} />;
            })}
          </g>

          {/* Kıyı sığlığı: kara siluetinin bulanık kopyası denizin altında */}
          <g className="map__shelf" filter="url(#mw-shelf)">
            {LAND_PATHS.map((d, i) => (
              <path key={i} d={d} />
            ))}
          </g>

          <g className="map__land">
            {LAND_PATHS.map((d, i) => (
              <path key={i} d={d} />
            ))}
          </g>

          <g className="map__seas">
            {SEA_LABELS.map((s) => {
              const { x, y } = project(s.lon, s.lat);
              return (
                <text key={s.text} x={x} y={y} textAnchor="middle">
                  {s.text}
                </text>
              );
            })}
          </g>

          {/* Ticaret yolları */}
          <g className="map__routes">
            {ROUTES.map(([a, b]) => {
              const from = cityAt(a);
              const to = cityAt(b);
              if (!from || !to) return null;
              const d = routePath(from, to);
              return (
                <g key={`${a}-${b}`}>
                  <path d={d} className="map__route-casing" />
                  <path d={d} className="map__route" />
                </g>
              );
            })}
          </g>

          {/* Yoldaki kervanlar */}
          <g className="map__caravans">
            {caravans.map((c) => {
              const pt = caravanPoint(c.from_city, c.to_city, c.progress);
              if (!pt) return null;
              const r = 2.2 + Math.min(c.cargo_units / 60, 2.2);
              return (
                <g key={c.id}>
                  <circle cx={pt.x} cy={pt.y} r={r + 2} className="map__caravan-halo" />
                  <circle cx={pt.x} cy={pt.y} r={r} className="map__caravan">
                    <title>{`${c.cargo_units} birim · ${c.from_city} → ${c.to_city}`}</title>
                  </circle>
                </g>
              );
            })}
          </g>

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

          <Compass />
        </svg>
      </div>

      <Legend />

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

/** Kuzey oku — haritanın gerçek bir çizim olduğunu söyleyen küçük detay. */
function Compass() {
  const { x, y } = project(37.4, 38.6);
  return (
    <g className="map__compass" transform={`translate(${x} ${y})`} aria-hidden>
      <circle r="15" />
      <path d="M0 -11 L3.5 2 L0 -1 L-3.5 2 Z" />
      <text y="-18" textAnchor="middle">
        K
      </text>
    </g>
  );
}

interface MarkerProps {
  city: CityState;
  selected: boolean;
  flash?: FeedItem;
  onSelect: () => void;
}

/** Etiketin daireye göre konumu — komşu şehirlerde çakışmayı önler. */
function labelPos(side: CityNode["labelSide"], r: number) {
  switch (side) {
    case "top":
      return { x: 0, y: -r - 9, anchor: "middle" as const };
    case "bottom":
      return { x: 0, y: r + 16, anchor: "middle" as const };
    case "left":
      return { x: -r - 7, y: 4, anchor: "end" as const };
    default:
      return { x: r + 7, y: 4, anchor: "start" as const };
  }
}

function CityMarker({ city, selected, flash, onSelect }: MarkerProps) {
  const { node, factoryCount, monopolyCount, warCount, chokeCount } = city;
  // Yarıçap sınai varlıkla büyür ama okunur bir bantta kalır.
  const r = 9 + Math.min(factoryCount * 1.3, 11);
  const state = warCount > 0 ? "war" : monopolyCount > 0 ? "monopoly" : "calm";
  const lp = labelPos(node.labelSide, r);

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
      {warCount > 0 && <circle r={r + 9} className="map__pulse" />}
      <circle r={r} className="map__disc" />
      <circle r={r} className="map__ring" />
      <text className="map__count num" y={4} textAnchor="middle">
        {factoryCount}
      </text>

      {monopolyCount > 0 && (
        <text className="map__crown" y={-r - 3} textAnchor="middle">
          {"♦".repeat(Math.min(monopolyCount, 3))}
        </text>
      )}
      {chokeCount > 0 && (
        <text className="map__lock" x={r + 2} y={-r + 2} textAnchor="middle">
          ⛔
        </text>
      )}

      <text className="map__label" x={lp.x} y={lp.y} textAnchor={lp.anchor}>
        {node.label}
      </text>

      {flash && (
        <text
          className="map__flash-text"
          x={lp.anchor === "end" ? -r - 7 : lp.anchor === "start" ? r + 7 : 0}
          y={node.labelSide === "bottom" ? -r - 8 : r + 16}
          textAnchor={lp.anchor}
        >
          {styleFor(flash.kind).icon} {styleFor(flash.kind).label}
        </text>
      )}
    </g>
  );
}

/** Harita göstergeleri — semboller yazısız okunmuyordu. */
function Legend() {
  return (
    <ul className="map__legend">
      <li>
        <span className="map__key map__key--monopoly" /> tekel
      </li>
      <li>
        <span className="map__key map__key--war" /> fiyat savaşı
      </li>
      <li>
        <span className="map__key map__key--choke" /> tedarik kesik
      </li>
      <li>
        <span className="map__key map__key--caravan" /> kervan
      </li>
      <li className="map__legend-note">daire içindeki sayı: fabrika</li>
    </ul>
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
        {city.node.label}'da henüz şirket yok.
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
                {f.monopolies > 0 && <span aria-hidden>♦ </span>}
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
