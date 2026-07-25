/**
 * Ürün ve şehir kataloğu — backend `ProductKind`/`CityId` ile birebir.
 *
 * Tek kaynak: eskiden bu listeler beş ayrı dosyada kopyalanıyordu ve
 * katalog büyüdüğünde (6 → 12 ürün) hepsi bayatlıyordu.
 */

export interface ProductInfo {
  slug: string;
  label: string;
  /** Üretim zincirindeki derinlik: 0 ham, 1-3 işlenmiş. */
  tier: 0 | 1 | 2 | 3;
}

export const PRODUCTS: readonly ProductInfo[] = [
  { slug: "pamuk", label: "Pamuk", tier: 0 },
  { slug: "bugday", label: "Buğday", tier: 0 },
  { slug: "zeytin", label: "Zeytin", tier: 0 },
  { slug: "boya", label: "Boya", tier: 0 },
  { slug: "uzum", label: "Üzüm", tier: 0 },
  { slug: "kumas", label: "Kumaş", tier: 1 },
  { slug: "un", label: "Un", tier: 1 },
  { slug: "zeytinyagi", label: "Zeytinyağı", tier: 1 },
  { slug: "sarap", label: "Şarap", tier: 1 },
  { slug: "elbise", label: "Elbise", tier: 2 },
  { slug: "ekmek", label: "Ekmek", tier: 2 },
  { slug: "ziyafet", label: "Ziyafet Sofrası", tier: 3 },
];

export const PRODUCT_SLUGS: readonly string[] = PRODUCTS.map((p) => p.slug);
export const RAW_SLUGS: readonly string[] = PRODUCTS.filter((p) => p.tier === 0).map(
  (p) => p.slug,
);
export const FINISHED_SLUGS: readonly string[] = PRODUCTS.filter(
  (p) => p.tier > 0,
).map((p) => p.slug);

const PRODUCT_BY_SLUG = new Map(PRODUCTS.map((p) => [p.slug, p]));

export function productLabel(slug: string): string {
  return PRODUCT_BY_SLUG.get(slug)?.label ?? slug;
}

export function productTier(slug: string): number {
  return PRODUCT_BY_SLUG.get(slug)?.tier ?? 0;
}

/** Eski `Record<string, string>` kullanan yerler için hazır sözlük. */
export const PRODUCT_LABEL: Record<string, string> = Object.fromEntries(
  PRODUCTS.map((p) => [p.slug, p.label]),
);

export const CITIES: readonly { slug: string; label: string }[] = [
  { slug: "istanbul", label: "İstanbul" },
  { slug: "ankara", label: "Ankara" },
  { slug: "izmir", label: "İzmir" },
  { slug: "bursa", label: "Bursa" },
  { slug: "konya", label: "Konya" },
];

export const CITY_SLUGS: readonly string[] = CITIES.map((c) => c.slug);

export const CITY_LABEL: Record<string, string> = Object.fromEntries(
  CITIES.map((c) => [c.slug, c.label]),
);

export function cityLabel(slug: string): string {
  return CITY_LABEL[slug] ?? slug;
}
