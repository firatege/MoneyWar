import type { Snapshot } from "../../types";

/**
 * Harita düzlemi. Koordinatlar Türkiye'nin kabaca coğrafi düzenini izler
 * ama ölçekli değil — okunabilirlik önce gelir: şehirler birbirine
 * yapışmasın, rotalar kesişmesin.
 */
export const MAP_W = 720;
export const MAP_H = 420;

export interface CityNode {
  slug: string;
  label: string;
  x: number;
  y: number;
}

export const CITIES: readonly CityNode[] = [
  { slug: "istanbul", label: "İstanbul", x: 168, y: 92 },
  { slug: "bursa", label: "Bursa", x: 232, y: 208 },
  { slug: "ankara", label: "Ankara", x: 494, y: 128 },
  { slug: "izmir", label: "İzmir", x: 128, y: 318 },
  { slug: "konya", label: "Konya", x: 484, y: 312 },
];

const BY_SLUG = new Map(CITIES.map((c) => [c.slug, c]));

export function cityAt(slug: string | null | undefined): CityNode | undefined {
  return slug ? BY_SLUG.get(slug) : undefined;
}

/** Kervanların izlediği rotalar — komşu şehirler arası yollar. */
export const ROUTES: readonly [string, string][] = [
  ["istanbul", "bursa"],
  ["istanbul", "ankara"],
  ["bursa", "izmir"],
  ["bursa", "ankara"],
  ["izmir", "konya"],
  ["ankara", "konya"],
];

/** Bir şehirde faaliyet gösteren firmanın haritadaki özeti. */
export interface CityFirm {
  id: number;
  name: string;
  /** Bu şehirdeki fabrika sayısı — rozet boyutunu belirler. */
  factories: number;
  /** Bu şehirde tekelinde tuttuğu pazar sayısı (taç). */
  monopolies: number;
  /** Bu şehirde bir pazarda savaşıyor mu? */
  atWar: boolean;
  /** Tedariki kesilmiş (fabrikası aç) mı? */
  choked: boolean;
}

export interface CityState {
  node: CityNode;
  firms: CityFirm[];
  /** Şehirdeki toplam fabrika — düğüm boyutu. */
  factoryCount: number;
  /** İlan edilmiş tekel sayısı — şehir renklenmesi. */
  monopolyCount: number;
  warCount: number;
  chokeCount: number;
}

/**
 * Snapshot'ı haritanın çizebileceği şehir durumlarına indirger.
 * Firma varlığı fabrikalardan gelir: bir şehirde fabrikası olan firma
 * orada "var" sayılır ve rozeti fabrika sayısıyla büyür.
 */
export function deriveCityStates(snapshot: Snapshot | null): CityState[] {
  const nameById = new Map<number, string>();
  for (const p of snapshot?.leaderboard ?? []) nameById.set(p.id, p.name);

  return CITIES.map((node) => {
    const firms = new Map<number, CityFirm>();
    const ensure = (id: number): CityFirm => {
      let f = firms.get(id);
      if (!f) {
        f = {
          id,
          name: nameById.get(id) ?? `#${id}`,
          factories: 0,
          monopolies: 0,
          atWar: false,
          choked: false,
        };
        firms.set(id, f);
      }
      return f;
    };

    for (const fac of snapshot?.factories ?? []) {
      if (fac.city === node.slug) ensure(fac.owner).factories += 1;
    }

    let monopolyCount = 0;
    for (const m of snapshot?.intrigue?.monopolies ?? []) {
      if (m.city !== node.slug) continue;
      ensure(m.firm_id).monopolies += 1;
      if (m.announced) monopolyCount += 1;
    }

    let warCount = 0;
    for (const w of snapshot?.intrigue?.price_wars ?? []) {
      if (w.city !== node.slug) continue;
      warCount += 1;
      ensure(w.attacker_id).atWar = true;
      ensure(w.target_id).atWar = true;
    }

    let chokeCount = 0;
    for (const c of snapshot?.intrigue?.supply_chokes ?? []) {
      if (c.city !== node.slug) continue;
      chokeCount += 1;
      ensure(c.victim_id).choked = true;
    }

    const list = [...firms.values()].sort(
      (a, b) =>
        b.monopolies - a.monopolies ||
        b.factories - a.factories ||
        a.id - b.id,
    );

    return {
      node,
      firms: list,
      factoryCount: list.reduce((n, f) => n + f.factories, 0),
      monopolyCount,
      warCount,
      chokeCount,
    };
  });
}

/** Yoldaki bir kervanın harita koordinatı (rota üstünde ilerleme oranıyla). */
export function caravanPoint(
  from: string | null,
  to: string | null,
  progress: number | null,
): { x: number; y: number } | null {
  const a = cityAt(from);
  const b = cityAt(to);
  if (!a || !b || progress == null) return null;
  const t = Math.min(Math.max(progress, 0), 1);
  return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
}
