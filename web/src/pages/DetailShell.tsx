import type { ReactNode } from "react";
import "./detail.css";

/**
 * Detay sayfalarının ortak kabuğu — başlık, geri yolu, yükleme/hata durumu.
 *
 * Üç sayfanın da aynı iskeleti paylaşması sadece kod tekrarını değil, göz
 * yorgunluğunu da azaltıyor: kademe değiştirince başlık, kapatma düğmesi ve
 * içerik hep aynı yerde duruyor.
 */

interface ShellProps {
  eyebrow: string;
  title: string;
  meta?: ReactNode;
  loading: boolean;
  error: string | null;
  empty?: boolean;
  onClose: () => void;
  children: ReactNode;
}

export function DetailShell({
  eyebrow,
  title,
  meta,
  loading,
  error,
  empty,
  onClose,
  children,
}: ShellProps) {
  return (
    <section className="dt" aria-label={title}>
      <header className="dt__head">
        <div className="dt__id">
          <p className="dt__eyebrow">{eyebrow}</p>
          <h2 className="dt__title">{title}</h2>
        </div>
        {meta && <div className="dt__meta">{meta}</div>}
        <button type="button" className="dt__close" onClick={onClose} aria-label="Haritaya dön">
          ← harita <kbd>Esc</kbd>
        </button>
      </header>

      <div className="dt__body">
        {error ? (
          <p className="dt__state dt__state--err">veri alınamadı — {error}</p>
        ) : loading ? (
          <p className="dt__state">yükleniyor…</p>
        ) : empty ? (
          <p className="dt__state">kayıt yok</p>
        ) : (
          children
        )}
      </div>
    </section>
  );
}

/** Tek bir sayı + etiketi. Grafiğe gerek olmayan yer. */
export function Stat({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  tone?: "gain" | "loss";
}) {
  return (
    <div className="dt__stat">
      <p className="dt__stat-label">{label}</p>
      <p className={`dt__stat-value${tone ? ` is-${tone}` : ""}`}>{value}</p>
      {sub && <p className="dt__stat-sub">{sub}</p>}
    </div>
  );
}

/** Başlıklı bölüm. */
export function Block({
  title,
  note,
  children,
  wide,
}: {
  title: string;
  note?: string;
  children: ReactNode;
  wide?: boolean;
}) {
  return (
    <section className={`dt__block${wide ? " dt__block--wide" : ""}`}>
      <h3 className="dt__block-title">
        {title}
        {note && <span className="dt__block-note">{note}</span>}
      </h3>
      {children}
    </section>
  );
}

/**
 * Sıralı oran listesi — "en çok kim / en çok ne" sorularının biçimi.
 *
 * Bir pasta grafiği yerine bu var: pastada dilimleri karşılaştırmak için
 * göz açı ölçmek zorunda; burada hepsi aynı taban çizgisinden başlıyor ve
 * etiket doğrudan yanında duruyor.
 */
export function RankList({
  rows,
  emptyText = "kayıt yok",
}: {
  rows: { key: string; label: ReactNode; value: number; display: string; color?: string }[];
  emptyText?: string;
}) {
  if (rows.length === 0) return <p className="dt__state">{emptyText}</p>;
  const max = Math.max(...rows.map((r) => Math.abs(r.value)), 1);
  return (
    <ul className="dt__ranks">
      {rows.map((r) => (
        <li key={r.key} className="dt__rank">
          <span className="dt__rank-label">{r.label}</span>
          <span className="dt__rank-track" aria-hidden="true">
            <span
              className="dt__rank-fill"
              style={{
                width: `${(Math.abs(r.value) / max) * 100}%`,
                background: r.color ?? "var(--accent-dim)",
              }}
            />
          </span>
          <span className="dt__rank-value">{r.display}</span>
        </li>
      ))}
    </ul>
  );
}
