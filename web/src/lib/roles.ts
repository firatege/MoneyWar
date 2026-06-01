// NpcKind etiketi → renk değişkeni + kısa kod eşlemesi.
// Etiketler backend NpcKind::label() ile birebir (Türkçe).

interface RoleStyle {
  varName: string;
  code: string; // rozet kısa kodu
}

const ROLE_MAP: Record<string, RoleStyle> = {
  Tüccar: { varName: "--role-tuccar", code: "TÜC" },
  Sanayici: { varName: "--role-sanayici", code: "SAN" },
  Alıcı: { varName: "--role-alici", code: "ALI" },
  Toptancı: { varName: "--role-toptanci", code: "TOP" },
  Spekülatör: { varName: "--role-spekulator", code: "SPK" },
  Çiftçi: { varName: "--role-ciftci", code: "ÇİF" },
  Banka: { varName: "--role-banka", code: "BNK" },
};

const FALLBACK: RoleStyle = { varName: "--text-faint", code: "—" };

export function roleColor(kind: string | null): string {
  const style = (kind && ROLE_MAP[kind]) || FALLBACK;
  return `var(${style.varName})`;
}

export function roleCode(kind: string | null): string {
  return ((kind && ROLE_MAP[kind]) || FALLBACK).code;
}
