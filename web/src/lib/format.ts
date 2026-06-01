// Sayı / para biçimlendirme yardımcıları (tr-TR).

const trGroup = new Intl.NumberFormat("tr-TR", { maximumFractionDigits: 0 });
const tr2 = new Intl.NumberFormat("tr-TR", {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

/** Tam lira, binlik ayraçlı: 165523 → "165.523". */
export function lira(n: number): string {
  return trGroup.format(Math.round(n));
}

/** 2 ondalıklı: 5.28 → "5,28". */
export function lira2(n: number | null | undefined): string {
  if (n == null) return "—";
  return tr2.format(n);
}

/** Kompakt büyük sayı: 165523 → "165,5B" (B = bin), 1_240_000 → "1,24Mn". */
export function compact(n: number): string {
  const abs = Math.abs(n);
  if (abs >= 1_000_000) return (n / 1_000_000).toFixed(2).replace(".", ",") + "Mn";
  if (abs >= 1_000) return (n / 1_000).toFixed(1).replace(".", ",") + "B";
  return trGroup.format(Math.round(n));
}

/** İşaretli kompakt PnL: +165,5B / -12,3B. */
export function signedCompact(n: number): string {
  const s = compact(Math.abs(n));
  if (n > 0) return "+" + s;
  if (n < 0) return "−" + s;
  return s;
}

/** Saniyeyi mm:ss formatına çevir. */
export function clock(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${r.toString().padStart(2, "0")}`;
}

/** Tick numarasını terminal etiketi olarak: 22 → "t22". */
export function tickLabel(t: number): string {
  return "t" + t.toString().padStart(2, "0");
}
