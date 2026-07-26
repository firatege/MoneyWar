import { useMemo } from "react";
import type { Snapshot } from "../../types";
import { CITIES } from "../../lib/catalog";
import "./network-map.css";

/**
 * Şehir ağı — beş düğüm, hepsi birbirine bağlı.
 *
 * Coğrafi harita bırakıldı: ekranın en büyük alanını kaplayıp en az şeyi
 * anlatıyordu. Şehirlerin İstanbul'un solunda mı sağında mı olduğu bu
 * oyunda hiçbir kurala girmiyor — her şehir her şehirle doğrudan ticaret
 * yapabiliyor. Düzenli beşgen bunu doğru anlatıyor: her yol görünür, hiçbiri
 * ötekinden uzun değil.
 *
 * Düğümün içindeki sayı fabrika adedi; çevresindeki halka bunların ne
 * kadarının çalıştığını gösterir. Yolun kalınlığı üstündeki kervan sayısı.
 */

// Düğümler yolları örtmemeli: ilk denemede daireler o kadar büyüktü ki
// komşu şehirler arasındaki kenar neredeyse tamamen dairelerin altında
// kalıyordu — "her yol birbirine bağlı" fikri görünmüyordu. Komşu merkezler
// arası mesafe 2·R·sin(36°) ≈ 94; iki yarıçap + halka bundan belirgin
// küçük kalsın diye düğüm küçültüldü.
const R = 80; // düğüm merkezlerinin yerleştiği çember yarıçapı
const NODE_R = 22; // düğüm dairesinin yarıçapı
const RING_W = 4; // dış halka kalınlığı

interface NodeGeom {
  slug: string;
  label: string;
  x: number;
  y: number;
}

/** Beşgen yerleşim — ilk düğüm tepede, saat yönünde. */
function pentagon(): NodeGeom[] {
  const n = CITIES.length;
  return CITIES.map((c, i) => {
    const a = (-90 + (360 / n) * i) * (Math.PI / 180);
    return { slug: c.slug, label: c.label, x: R * Math.cos(a), y: R * Math.sin(a) };
  });
}

/**
 * Dar mod: düğümler tek sıra.
 *
 * Beşgen kare bir alan ister; detay paneli açıkken harita alçak ve geniş
 * bir şeride iniyor. O şeride beşgeni sığdırmak SVG'nin tamamını
 * küçültüyor ve şehir adları okunmaz oluyordu. Sırada yükseklik gereksiz
 * yere harcanmıyor, yazı boyutu korunuyor.
 *
 * Sırada yollar çizilmiyor: eş doğrusal noktalar arasındaki on kenar
 * üst üste binip anlamsız bir bulamaç olurdu. "Her yol bağlı" bilgisi
 * geniş görünümde zaten kuruluyor.
 */
function row(): NodeGeom[] {
  const gap = 50;
  const offset = ((CITIES.length - 1) * gap) / 2;
  return CITIES.map((c, i) => ({
    slug: c.slug,
    label: c.label,
    x: i * gap - offset,
    y: 0,
  }));
}

interface Props {
  snapshot: Snapshot | null;
  selected: string | null;
  onSelect: (city: string) => void;
  /**
   * Altında detay paneli açıkken harita şeride sıkışır. SVG tek parça
   * ölçeklendiği için yazılar da küçülür ve "3 çalışıyor" alt satırı
   * okunmaz hale gelir. Dar modda o satır düşer; sayı ve şehir adı kalır.
   */
  compact?: boolean;
}

export function NetworkMap({ snapshot, selected, onSelect, compact = false }: Props) {
  const nodes = useMemo(() => (compact ? row() : pentagon()), [compact]);
  const nodeR = compact ? 17 : NODE_R;
  const byslug = useMemo(() => new Map(nodes.map((n) => [n.slug, n])), [nodes]);

  const stats = useMemo(() => {
    const m = new Map<string, { total: number; idle: number; farms: number }>();
    for (const n of nodes) m.set(n.slug, { total: 0, idle: 0, farms: 0 });
    for (const f of snapshot?.factories ?? []) {
      const e = m.get(f.city);
      if (!e) continue;
      e.total += 1;
      if (f.idle) e.idle += 1;
    }
    for (const f of snapshot?.private_farms ?? []) {
      const e = m.get(f.city);
      if (e) e.farms += 1;
    }
    return m;
  }, [snapshot, nodes]);

  /** Tekel tutulan şehirler — düğümün üstünde işaret. */
  const monopolyCities = useMemo(() => {
    const s = new Set<string>();
    for (const m of snapshot?.intrigue.monopolies ?? []) s.add(m.city);
    return s;
  }, [snapshot]);

  /** Yol → üstündeki kervan sayısı. Anahtar alfabetik sıralı çift. */
  const traffic = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of snapshot?.caravans ?? []) {
      if (!c.from_city || !c.to_city) continue;
      const key = [c.from_city, c.to_city].sort().join("|");
      m.set(key, (m.get(key) ?? 0) + 1);
    }
    return m;
  }, [snapshot]);

  const edges = useMemo(() => {
    const out: { a: NodeGeom; b: NodeGeom; key: string; load: number }[] = [];
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const key = [nodes[i].slug, nodes[j].slug].sort().join("|");
        out.push({ a: nodes[i], b: nodes[j], key, load: traffic.get(key) ?? 0 });
      }
    }
    return out;
  }, [nodes, traffic]);

  /** Yoldaki kervanlar — ilerleme oranına göre çizgi üstünde bir nokta. */
  const movers = useMemo(() => {
    const out: { id: number; x: number; y: number }[] = [];
    for (const c of snapshot?.caravans ?? []) {
      if (!c.from_city || !c.to_city || c.progress == null) continue;
      const a = byslug.get(c.from_city);
      const b = byslug.get(c.to_city);
      if (!a || !b) continue;
      out.push({
        id: c.id,
        x: a.x + (b.x - a.x) * c.progress,
        y: a.y + (b.y - a.y) * c.progress,
      });
    }
    return out;
  }, [snapshot, byslug]);

  const maxLoad = Math.max(1, ...edges.map((e) => e.load));

  return (
    <section
      className={`netmap${compact ? " netmap--compact" : ""}`}
      aria-labelledby="netmap-title"
    >
      <header className="netmap__head">
        <h2 id="netmap-title" className="netmap__title">
          Şehir ağı
        </h2>
        <p className="netmap__hint">
          {compact
            ? "başka şehre tıkla"
            : "Daire içindeki sayı fabrika adedi · halka çalışanları gösterir · şehre tıkla"}
        </p>
      </header>

      <svg
        className="netmap__svg"
        viewBox={compact ? "-140 -26 280 62" : "-116 -110 232 238"}
        role="group"
        aria-label="Şehirler ve aralarındaki yollar"
      >
        {/* Yollar — hepsi hep görünür, trafik olan kalınlaşır. */}
        {!compact && (
        <g className="netmap__edges">
          {edges.map((e) => (
            <line
              key={e.key}
              x1={e.a.x}
              y1={e.a.y}
              x2={e.b.x}
              y2={e.b.y}
              className={`netmap__edge${e.load > 0 ? " netmap__edge--busy" : ""}`}
              strokeWidth={0.7 + (e.load / maxLoad) * 2.4}
            />
          ))}
        </g>
        )}

        {/* Yoldaki kervanlar. */}
        {!compact && (
        <g className="netmap__movers">
          {movers.map((m) => (
            <circle key={m.id} cx={m.x} cy={m.y} r={2.2} className="netmap__mover" />
          ))}
        </g>
        )}

        {/* Şehirler. */}
        {nodes.map((n) => {
          const s = stats.get(n.slug) ?? { total: 0, idle: 0, farms: 0 };
          const active = s.total - s.idle;
          const isSel = selected === n.slug;
          // Halka: çalışan fabrikaların payı kadar dolu yay.
          const circ = 2 * Math.PI * (nodeR + RING_W / 2 + 1);
          const filled = s.total > 0 ? (active / s.total) * circ : 0;
          return (
            <g
              key={n.slug}
              className={`netmap__node${isSel ? " netmap__node--sel" : ""}`}
              transform={`translate(${n.x} ${n.y})`}
              role="button"
              tabIndex={0}
              aria-label={`${n.label}: ${s.total} fabrika, ${active} çalışıyor`}
              onClick={() => onSelect(n.slug)}
              onKeyDown={(ev) => {
                if (ev.key === "Enter" || ev.key === " ") {
                  ev.preventDefault();
                  onSelect(n.slug);
                }
              }}
            >
              <circle r={nodeR} className="netmap__disc" />
              {/* Halka zemini + çalışan payı. Yay tepeden başlasın diye döndürülür. */}
              <circle
                r={nodeR + RING_W / 2 + 1}
                className="netmap__ring-bg"
                strokeWidth={RING_W}
              />
              {s.total > 0 && (
                <circle
                  r={nodeR + RING_W / 2 + 1}
                  className="netmap__ring"
                  strokeWidth={RING_W}
                  strokeDasharray={`${filled} ${circ - filled}`}
                  transform="rotate(-90)"
                />
              )}
              {monopolyCities.has(n.slug) && (
                <circle cy={-nodeR - 6} r={2.6} className="netmap__flag" />
              )}
              <text className="netmap__count" y={1}>
                {s.total}
              </text>
              {!compact && (
                <text className="netmap__sub" y={11}>
                  {s.total > 0 ? `${active} çalışıyor` : "fabrika yok"}
                </text>
              )}
              <text className="netmap__label" y={nodeR + 12}>
                {n.label}
              </text>
            </g>
          );
        })}
      </svg>
    </section>
  );
}
