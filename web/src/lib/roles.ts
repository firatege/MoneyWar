// NpcKind etiketi → renk değişkeni + kısa kod eşlemesi.
// Etiketler backend NpcKind::label() ile birebir (Türkçe).

interface RoleStyle {
  /** İşaret rengi — çubuk, nokta, halka. Yüzeye karşı 3:1 doğrulandı. */
  varName: string;
  /** Yazı rengi — aynı kimlik, daha açık ton. 4.5:1 eşiğini geçer. */
  inkName: string;
  code: string; // rozet kısa kodu
}

const ROLE_MAP: Record<string, RoleStyle> = {
  Tüccar: { varName: "--role-tuccar", inkName: "--ink-tuccar", code: "TÜC" },
  Sanayici: { varName: "--role-sanayici", inkName: "--ink-sanayici", code: "SAN" },
  Alıcı: { varName: "--role-alici", inkName: "--ink-alici", code: "ALI" },
  Toptancı: { varName: "--role-toptanci", inkName: "--ink-toptanci", code: "TOP" },
  Spekülatör: { varName: "--role-spekulator", inkName: "--ink-spekulator", code: "SPK" },
  Çiftçi: { varName: "--role-ciftci", inkName: "--ink-ciftci", code: "ÇİF" },
  Banka: { varName: "--role-banka", inkName: "--ink-banka", code: "BNK" },
};

const FALLBACK: RoleStyle = {
  varName: "--text-faint",
  inkName: "--text-dim",
  code: "—",
};

/** İşaret rengi — çubuk, nokta, halka gibi dolgular için. */
export function roleColor(kind: string | null): string {
  const style = (kind && ROLE_MAP[kind]) || FALLBACK;
  return `var(${style.varName})`;
}

/**
 * Yazı rengi. Dolgu rengi metinde kullanılmamalı: palet 3:1 işaret eşiğine
 * göre seçildi, yazı 4.5:1 ister ve koyu tonlar bunu geçmiyor.
 */
export function roleInk(kind: string | null): string {
  const style = (kind && ROLE_MAP[kind]) || FALLBACK;
  return `var(${style.inkName})`;
}

export function roleCode(kind: string | null): string {
  return ((kind && ROLE_MAP[kind]) || FALLBACK).code;
}
