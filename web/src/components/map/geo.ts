import type { Snapshot } from "../../types";

/**
 * Harita coğrafyası — gerçek enlem/boylam üstüne kurulu.
 *
 * Şehirler gerçek koordinatlarında durur ve kıyı çizgisi Batı Anadolu'nun
 * gerçek hatlarını izler (Boğaz, Marmara, Çanakkale, Ege girintileri,
 * Akdeniz). Böylece harita "5 daire ve çizgi" değil, tanınabilir bir bölge
 * haritası gibi okunur.
 *
 * Projeksiyon: basit eş-dikdörtgen (equirectangular), boylam ekseninde
 * `cos(39°)` düzeltmesiyle. Bu ölçekte Türkiye için mesafe bozulması gözle
 * fark edilmez ve hesabı okunur tutar.
 */

/** Görünen pencere: batı sınırı ve kuzey sınırı (derece). */
const LON_0 = 25.5;
const LAT_0 = 42.3;
/** Derece başına piksel. Boylam ekseni enlem daralmasıyla düzeltilir. */
const PX_PER_DEG = 60;
const LON_SCALE = PX_PER_DEG * Math.cos((39 * Math.PI) / 180); // ≈ 46.6

export const MAP_W = 606; // ≈ 38.5°D boylamına kadar
export const MAP_H = 390; // ≈ 35.8°K enlemine kadar

/** Coğrafi koordinatı harita düzlemine taşı. */
export function project(lon: number, lat: number): { x: number; y: number } {
  return { x: (lon - LON_0) * LON_SCALE, y: (LAT_0 - lat) * PX_PER_DEG };
}

/**
 * `[lon, lat]` dizisini yumuşatılmış kapalı SVG path'ine çevirir.
 *
 * Düz çizgi birleştirme kıyıyı "low-poly" bir çokgene çeviriyordu. Kapalı
 * Catmull-Rom eğrisi kübik Bézier'e dönüştürülüyor: az sayıda gerçek
 * koordinatla akıcı, doğal görünen bir kıyı çizgisi çıkıyor.
 */
function toPath(points: readonly [number, number][]): string {
  const pts = points.map(([lon, lat]) => project(lon, lat));
  const n = pts.length;
  if (n < 3) return "";
  const at = (i: number) => pts[((i % n) + n) % n];
  const f = (v: number) => v.toFixed(1);

  let d = `M${f(pts[0].x)} ${f(pts[0].y)}`;
  for (let i = 0; i < n; i++) {
    const p0 = at(i - 1);
    const p1 = at(i);
    const p2 = at(i + 1);
    const p3 = at(i + 2);
    // Catmull-Rom → Bézier (gerilim 1/6, standart dönüşüm).
    const c1x = p1.x + (p2.x - p0.x) / 6;
    const c1y = p1.y + (p2.y - p0.y) / 6;
    const c2x = p2.x - (p3.x - p1.x) / 6;
    const c2y = p2.y - (p3.y - p1.y) / 6;
    d += ` C${f(c1x)} ${f(c1y)} ${f(c2x)} ${f(c2y)} ${f(p2.x)} ${f(p2.y)}`;
  }
  return `${d} Z`;
}

/**
 * Anadolu kıyı çizgisi — Boğaz'ın Asya yakasından başlar, Karadeniz'i
 * doğuya izler, haritanın doğu kenarından iner, Akdeniz ve Ege'yi batıya
 * takip eder, Çanakkale'den Marmara'nın güney kıyısına döner.
 */
const ANATOLIA: readonly [number, number][] = [
  [29.05, 41.02], // Boğaz — Anadolu yakası
  [30.3, 41.15],
  [31.4, 41.3],
  [32.3, 41.9], // Karadeniz çıkıntısı (İnebolu)
  [33.8, 42.05], // Sinop burnu
  [35.2, 41.7],
  [36.4, 41.35], // Samsun
  [38.9, 41.2], // harita dışına taşar
  [38.9, 36.2],
  [36.4, 36.5], // İskenderun
  [35.0, 36.6],
  [34.0, 36.3],
  [33.0, 36.2], // Silifke
  [32.0, 36.5],
  [31.0, 36.3],
  [30.4, 36.25], // Antalya
  [29.7, 36.2],
  [29.1, 36.25], // Fethiye
  [28.5, 36.6],
  [28.1, 36.72], // Marmaris
  [27.5, 36.8],
  [27.28, 37.05], // Bodrum yarımadası
  [27.75, 37.35],
  [27.25, 37.62], // Kuşadası
  [27.05, 38.0],
  [26.4, 38.22], // Çeşme yarımadası
  [27.15, 38.45], // İzmir körfezi
  [26.85, 38.65],
  [26.72, 39.0],
  [26.6, 39.3],
  [26.95, 39.55], // Edremit körfezi
  [26.3, 39.52],
  [26.2, 39.9],
  [26.25, 40.15], // Çanakkale — Asya yakası
  [27.2, 40.36], // Marmara güney kıyısı
  [28.0, 40.36],
  [28.9, 40.42],
  [29.45, 40.7],
  [29.95, 40.76], // İzmit körfezi
  [29.3, 40.8],
];

/** Trakya — Boğaz ile Çanakkale arasında kalan Avrupa yakası. */
const THRACE: readonly [number, number][] = [
  [26.1, 40.15], // Çanakkale — Avrupa yakası
  [26.05, 40.6],
  [26.3, 40.95],
  [26.6, 41.35],
  [27.1, 41.62], // kuzeybatı sınır
  [28.1, 41.45],
  [28.95, 41.25], // Boğaz — Karadeniz ağzı
  [28.72, 41.02],
  [28.2, 40.97],
  [27.5, 40.96],
  [27.0, 40.72],
  [26.7, 40.46], // Marmara kuzey kıyısı, Gelibolu'ya iner
];

export const LAND_PATHS: readonly string[] = [toPath(ANATOLIA), toPath(THRACE)];

/** Deniz adları — gerçek haritaların en güçlü okunabilirlik ipucu. */
export const SEA_LABELS: readonly { text: string; lon: number; lat: number }[] = [
  { text: "KARADENİZ", lon: 30.6, lat: 42.0 },
  { text: "MARMARA", lon: 28.0, lat: 40.72 },
  { text: "EGE DENİZİ", lon: 26.0, lat: 38.4 },
  { text: "AKDENİZ", lon: 29.6, lat: 35.95 },
];

export interface CityNode {
  slug: string;
  label: string;
  /** Gerçek coğrafi konum. */
  lon: number;
  lat: number;
  /** Projeksiyon sonucu harita koordinatı. */
  x: number;
  y: number;
  /** Etiketin daireye göre yönü — komşu etiketler çakışmasın. */
  labelSide: "top" | "bottom" | "left" | "right";
}

function city(
  slug: string,
  label: string,
  lon: number,
  lat: number,
  labelSide: CityNode["labelSide"],
): CityNode {
  return { slug, label, lon, lat, ...project(lon, lat), labelSide };
}

export const CITIES: readonly CityNode[] = [
  city("istanbul", "İstanbul", 28.98, 41.01, "top"),
  city("bursa", "Bursa", 29.06, 40.19, "left"),
  city("ankara", "Ankara", 32.85, 39.93, "top"),
  city("izmir", "İzmir", 27.14, 38.42, "left"),
  city("konya", "Konya", 32.48, 37.87, "bottom"),
];

const BY_SLUG = new Map(CITIES.map((c) => [c.slug, c]));

export function cityAt(slug: string | null | undefined): CityNode | undefined {
  return slug ? BY_SLUG.get(slug) : undefined;
}

/** Kervanların izlediği karayolları — gerçek güzergâhlara yakın. */
export const ROUTES: readonly [string, string][] = [
  ["istanbul", "bursa"],
  ["istanbul", "ankara"],
  ["bursa", "izmir"],
  ["bursa", "ankara"],
  ["izmir", "konya"],
  ["ankara", "konya"],
];

/**
 * Rotayı hafifçe kavisli çizer. Düz çizgiler ağ diyagramı gibi duruyordu;
 * kavis yol hissi verir ve iki şehir arasındaki çift yönü ayırır.
 */
export function routePath(from: CityNode, to: CityNode): string {
  const mx = (from.x + to.x) / 2;
  const my = (from.y + to.y) / 2;
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const len = Math.hypot(dx, dy) || 1;
  // Dikey normal boyunca %6 sapma — abartısız bir yay.
  const bow = 0.06;
  const cx = mx - (dy / len) * len * bow;
  const cy = my + (dx / len) * len * bow;
  return `M${from.x.toFixed(1)} ${from.y.toFixed(1)} Q${cx.toFixed(1)} ${cy.toFixed(
    1,
  )} ${to.x.toFixed(1)} ${to.y.toFixed(1)}`;
}

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
  // Rota yayıyla aynı kuadratik eğri üzerinde ilerle.
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  const bow = 0.06;
  const cx = mx - (dy / len) * len * bow;
  const cy = my + (dx / len) * len * bow;
  const u = 1 - t;
  return {
    x: u * u * a.x + 2 * u * t * cx + t * t * b.x,
    y: u * u * a.y + 2 * u * t * cy + t * t * b.y,
  };
}
