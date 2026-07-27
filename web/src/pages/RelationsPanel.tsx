import { useMemo, useState } from "react";
import { useDetail } from "../hooks/useDetail";
import type { GraphEdgeDto, RelationsGraph } from "../types-detail";
import { compact, tickLabel } from "../lib/format";
import { roleColor, roleInk } from "../lib/roles";
import { Block, DetailShell, Stat } from "./DetailShell";
import "./relations.css";

/**
 * İlişki ağı sayfası — kim kiminle çalışıyor, kim kime düşman.
 *
 * # Neden akış yetmiyordu
 *
 * İlişki motorda iki yönlü işliyor: olay ilişkiyi doğuruyor (fiyat kırma →
 * kin) ve ilişki olayı etkiliyor (güven → daha yüksek teklif). Ama akışta
 * yalnız tek tek satırlar geçiyordu; "KİN" yazan bir satır görünüp
 * kayboluyor, kimin kime ne yaptığı hiç toplanmıyordu.
 *
 * # Neden bu biçim
 *
 * Kuvvet-yönlendirmeli (force-directed) graf her açılışta farklı yerleşir —
 * izleyici aynı firmayı iki kez aynı yerde bulamaz. Burada düğümler
 * **role göre gruplanmış sabit bir çember** üzerinde: aynı rolün firmaları
 * yan yana, konum tick'ten tick'e değişmiyor. Kenarlar merkeze doğru
 * kavislenen yaylar — düz çizgide 121 kenar birbirini kesip okunmaz hale
 * geliyordu.
 *
 * Fizik motoru ya da dış kütüphane yok: yerleşim tamamen deterministik.
 */

const R = 158;
/**
 * Odaklanılmadığında çizilen en güçlü ticaret bağı sayısı.
 *
 * Tüm bağları çizmek denendi ve okunmaz çıktı: 287 kenar merkeze doğru
 * kavislenince yumak oluyor, hiçbir ilişki seçilemiyor. Husumetler her
 * zaman tam gösteriliyor (zaten az ve asıl merak edilen); ticaret bağları
 * en güçlüden kırpılıyor. "hepsi" filtresi sınırı kaldırır.
 */
const TOP_TRADE_EDGES = 28;
const NODE_R_MIN = 5;
const NODE_R_MAX = 13;

/** Kenar türlerinin görsel kimliği ve okunur adı. */
const EDGE_KINDS: Record<string, { label: string; color: string; note: string }> = {
  ticaret: {
    label: "ticaret",
    color: "var(--gain-dim)",
    note: "birlikte iş yapıyorlar — kalınlık dönen paraya göre",
  },
  kin: {
    label: "kin",
    color: "var(--role-spekulator)",
    note: "zarar gören, zarar vereni hatırlıyor",
  },
  savas: {
    label: "fiyat savaşı",
    color: "var(--loss)",
    note: "karşılıklı fiyat kırma sürüyor",
  },
  bogma: {
    label: "tedarik boğma",
    color: "var(--role-alici)",
    note: "rakibin girdisi piyasadan çekiliyor",
  },
};

type Filter = "hepsi" | "ticaret" | "husumet";

interface Props {
  tick: number;
  onClose: () => void;
  onSelectFirm: (id: number) => void;
}

export function RelationsPanel({ tick, onClose, onSelectFirm }: Props) {
  const { data, error, loading } = useDetail<RelationsGraph>("/api/relations", tick);
  const [filter, setFilter] = useState<Filter>("hepsi");
  const [focused, setFocused] = useState<number | null>(null);

  /** Düğümler role göre gruplu, çember üzerinde sabit konumda. */
  const layout = useMemo(() => {
    const nodes = [...(data?.nodes ?? [])].sort((a, b) => {
      const r = (a.role ?? "").localeCompare(b.role ?? "", "tr");
      return r !== 0 ? r : a.id - b.id;
    });
    const maxFab = Math.max(1, ...nodes.map((n) => n.factories));
    const pos = new Map<number, { x: number; y: number; r: number; a: number }>();
    nodes.forEach((n, i) => {
      const a = (-90 + (360 / Math.max(1, nodes.length)) * i) * (Math.PI / 180);
      pos.set(n.id, {
        x: R * Math.cos(a),
        y: R * Math.sin(a),
        r: NODE_R_MIN + (n.factories / maxFab) * (NODE_R_MAX - NODE_R_MIN),
        a,
      });
    });
    return { nodes, pos };
  }, [data]);

  const edges = useMemo(() => {
    const all = data?.edges ?? [];
    // Odaklanılan firma varsa yalnız ona değen kenarlar — yüzlerce kenarda
    // tek firmayı izlemek başka türlü mümkün değil.
    if (focused != null) {
      return all.filter((e) => e.from === focused || e.to === focused);
    }
    const conflicts = all.filter((e) => e.kind !== "ticaret");
    const trades = [...all.filter((e) => e.kind === "ticaret")].sort(
      (a, b) => b.strength - a.strength,
    );
    if (filter === "husumet") return conflicts;
    // Husumet her zaman tam; ticaret en güçlüden kırpılıyor.
    const shownTrades = filter === "ticaret" ? trades : trades.slice(0, TOP_TRADE_EDGES);
    return filter === "ticaret" ? shownTrades : [...shownTrades, ...conflicts];
  }, [data, filter, focused]);

  /** Odaklanılan firmanın ilişkileri, açıklamalı liste hâlinde. */
  const focusedRows = useMemo(() => {
    if (focused == null || !data) return [];
    const nameOf = (id: number) => data.nodes.find((n) => n.id === id)?.name ?? `#${id}`;
    const roleOf = (id: number) => data.nodes.find((n) => n.id === id)?.role ?? null;
    return data.edges
      .filter((e) => e.from === focused || e.to === focused)
      .map((e) => {
        const other = e.from === focused ? e.to : e.from;
        return { edge: e, other, name: nameOf(other), role: roleOf(other) };
      })
      .sort((a, b) => {
        if (a.edge.kind === b.edge.kind) return b.edge.strength - a.edge.strength;
        return a.edge.kind === "ticaret" ? 1 : -1; // husumet üstte
      });
  }, [data, focused]);

  const focusedNode = data?.nodes.find((n) => n.id === focused) ?? null;

  return (
    <DetailShell
      eyebrow="İlişki ağı"
      title={focusedNode ? focusedNode.name : "Kim kiminle"}
      meta={
        data?.window_from_tick != null && (
          <span>
            {tickLabel(data.window_from_tick)}–{tickLabel(data.tick)}
          </span>
        )
      }
      loading={loading}
      error={error}
      onClose={onClose}
    >
      {data && (
        <>
          <div className="dt__stats">
            <Stat label="Ticaret bağı" value={data.summary.trade_edges} sub="birlikte iş yapan çift" />
            <Stat
              label="Husumet"
              value={data.summary.conflict_edges}
              tone={data.summary.conflict_edges > 0 ? "loss" : undefined}
              sub={`${data.summary.grudges} kin · ${data.summary.price_wars} savaş`}
            />
            <Stat label="Tedarik boğma" value={data.summary.supply_chokes} sub="süregelen" />
            <Stat label="Tekel" value={data.summary.monopolies} sub="elde tutulan pazar" />
            <Stat
              label="En bağlantılı"
              value={data.summary.most_connected?.name ?? "—"}
              sub="en çok ortağı olan"
            />
          </div>

          <div className="rel__layout">
            {/* ── Graf ────────────────────────────────────────────────── */}
            <div className="rel__graph">
              <div className="rel__toolbar">
                <div className="rel__filters" role="group" aria-label="İlişki türü">
                  {(["hepsi", "ticaret", "husumet"] as Filter[]).map((f) => (
                    <button
                      key={f}
                      type="button"
                      className={`rel__filter${filter === f ? " is-on" : ""}`}
                      aria-pressed={filter === f}
                      onClick={() => setFilter(f)}
                    >
                      {f}
                    </button>
                  ))}
                </div>
                {focused != null && (
                  <button type="button" className="rel__clear" onClick={() => setFocused(null)}>
                    ← tüm ağ
                  </button>
                )}
              </div>

              <svg
                className="rel__svg"
                viewBox="-236 -222 472 468"
                role="group"
                aria-label="Firmalar arası ilişki ağı"
              >
                {/* Kenarlar — merkeze doğru kavisli yay. Düz çizgide
                    yüzden fazla kenar birbirini kesip okunmaz oluyordu. */}
                <g className="rel__edges">
                  {edges.map((e, i) => {
                    const a = layout.pos.get(e.from);
                    const b = layout.pos.get(e.to);
                    if (!a || !b) return null;
                    const style = EDGE_KINDS[e.kind] ?? EDGE_KINDS.ticaret;
                    const isTrade = e.kind === "ticaret";
                    return (
                      <path
                        key={`${e.kind}-${e.from}-${e.to}-${i}`}
                        d={`M ${a.x} ${a.y} Q 0 0 ${b.x} ${b.y}`}
                        className={`rel__edge rel__edge--${e.kind}`}
                        stroke={style.color}
                        strokeWidth={isTrade ? 0.4 + e.strength * 2.2 : 1.6}
                        strokeOpacity={focused == null && isTrade ? 0.35 : 0.9}
                      >
                        <title>{e.label}</title>
                      </path>
                    );
                  })}
                </g>

                {/* Düğümler */}
                {layout.nodes.map((n) => {
                  const p = layout.pos.get(n.id);
                  if (!p) return null;
                  const sel = focused === n.id;
                  // Etiket çemberin dışında ve **teğet doğrultuda döndürülmüş**.
                  // Yatay yazıda 34 isim, özellikle sol yayda üst üste
                  // biniyordu; radyal dizilim isimleri birbirinden ayırıyor.
                  const deg = (p.a * 180) / Math.PI;
                  const flip = deg > 90 || deg < -90;
                  const lr = p.r + 8;
                  return (
                    <g
                      key={n.id}
                      className={`rel__node${sel ? " is-sel" : ""}`}
                      role="button"
                      tabIndex={0}
                      aria-label={`${n.name}: ${n.partners} ortak, ${n.rivals} husumet`}
                      onClick={() => setFocused(sel ? null : n.id)}
                      onKeyDown={(ev) => {
                        if (ev.key === "Enter" || ev.key === " ") {
                          ev.preventDefault();
                          setFocused(sel ? null : n.id);
                        }
                      }}
                    >
                      {/* Görünmez tıklama alanı. Daire 5-13px; hem parmakla
                          hem fareyle isabet ettirmek için hedef büyütülüyor.
                          Grubun kendisine güvenmek olmuyordu: etiket çemberin
                          dışında olduğu için grubun sınırlayıcı kutusu kocaman
                          çıkıyor ve merkezi boşluğa düşüyor. */}
                      <circle cx={p.x} cy={p.y} r={Math.max(p.r + 6, 11)} className="rel__hit" />
                      <circle cx={p.x} cy={p.y} r={p.r} className="rel__disc" fill={roleColor(n.role)} />
                      {n.monopolies > 0 && (
                        <circle cx={p.x} cy={p.y} r={p.r + 3} className="rel__crown" />
                      )}
                      <text
                        className="rel__label"
                        fill={sel ? "var(--text)" : "var(--text-faint)"}
                        // Sıra önemli: önce düğüme git, sonra yarıçap
                        // doğrultusuna dön, sonra **dışarı** kay. Sol yarıda
                        // yazı ters okunmasın diye 180° çevrilir; kaydırma
                        // yine dışarı doğru olmalı — negatif yazınca etiket
                        // dairenin içine düşüyordu.
                        transform={
                          `translate(${p.x} ${p.y}) rotate(${deg}) translate(${lr} 0)` +
                          (flip ? " rotate(180)" : "")
                        }
                        textAnchor={flip ? "end" : "start"}
                        dominantBaseline="middle"
                      >
                        {n.name.length > 16 ? `${n.name.slice(0, 15)}…` : n.name}
                      </text>
                    </g>
                  );
                })}
              </svg>

              <ul className="rel__legend">
                {Object.entries(EDGE_KINDS).map(([k, v]) => (
                  <li key={k}>
                    <i style={{ background: v.color }} />
                    <b>{v.label}</b> — {v.note}
                  </li>
                ))}
              </ul>
              <p className="rel__hint">
                Daire büyüklüğü fabrika sayısı · halka tekel · firmaya tıkla, yalnız onun
                ilişkileri kalsın
              </p>
            </div>

            {/* ── Açıklamalı liste ─────────────────────────────────────── */}
            <div className="rel__side">
              {focusedNode ? (
                <Block
                  title={focusedNode.name}
                  note={`${focusedNode.partners} ortak · ${focusedNode.rivals} husumet`}
                >
                  <p className="rel__who">
                    <span style={{ color: roleInk(focusedNode.role) }}>
                      {focusedNode.role ?? "—"}
                    </span>
                    <span className="dt__muted">
                      {" · "}
                      {focusedNode.factories} fabrika · {compact(focusedNode.pnl_lira)}₺
                    </span>
                    <button
                      type="button"
                      className="dt__linkbtn rel__open"
                      onClick={() => onSelectFirm(focusedNode.id)}
                    >
                      firma sayfası →
                    </button>
                  </p>
                  <ul className="rel__rows">
                    {focusedRows.map(({ edge, other, name, role }, i) => (
                      <li key={`${edge.kind}-${other}-${i}`} className="rel__row">
                        <span
                          className="rel__dot"
                          style={{ background: (EDGE_KINDS[edge.kind] ?? EDGE_KINDS.ticaret).color }}
                          aria-hidden="true"
                        />
                        <span className="rel__row-body">
                          <button
                            type="button"
                            className="dt__linkbtn rel__row-name"
                            style={{ color: roleInk(role) }}
                            onClick={() => setFocused(other)}
                          >
                            {name}
                          </button>
                          <span className="rel__row-label">{edge.label}</span>
                        </span>
                        <span className="rel__row-tag">
                          {(EDGE_KINDS[edge.kind] ?? EDGE_KINDS.ticaret).label}
                        </span>
                      </li>
                    ))}
                    {focusedRows.length === 0 && (
                      <li className="dt__state">bu firmanın kayıtlı ilişkisi yok</li>
                    )}
                  </ul>
                </Block>
              ) : (
                <Block title="Ağda ne var" note="bir firmaya tıkla">
                  <p className="rel__intro">
                    Her daire bir firma, her çizgi bir ilişki. İlişkiler kendiliğinden
                    doğuyor: birlikte iş yapmak <b>güven</b> biriktiriyor, fiyatı kırılan
                    firma <b>kin</b> tutuyor, kin tutan sonraki kararlarında sert
                    davranıyor. Yani olay ilişkiyi, ilişki de olayı besliyor.
                  </p>
                  {data.summary.fiercest_rivalry && (
                    <p className="rel__intro">
                      Şu an en sert husumet{" "}
                      <b>{data.summary.fiercest_rivalry[0].name}</b> ile{" "}
                      <b>{data.summary.fiercest_rivalry[1].name}</b> arasında.
                    </p>
                  )}
                  <TopEdges edges={data.edges} onFocus={setFocused} nodes={data.nodes} />
                </Block>
              )}
            </div>
          </div>
        </>
      )}
    </DetailShell>
  );
}

/** Ağın en güçlü bağları — sayfaya girer girmez bir yere bakılsın. */
function TopEdges({
  edges,
  nodes,
  onFocus,
}: {
  edges: GraphEdgeDto[];
  nodes: { id: number; name: string; role: string | null }[];
  onFocus: (id: number) => void;
}) {
  const nameOf = (id: number) => nodes.find((n) => n.id === id)?.name ?? `#${id}`;
  const conflicts = edges.filter((e) => e.kind !== "ticaret").slice(0, 5);
  const trades = [...edges.filter((e) => e.kind === "ticaret")]
    .sort((a, b) => b.strength - a.strength)
    .slice(0, 6);

  return (
    <>
      {conflicts.length > 0 && (
        <>
          <h4 className="rel__subtitle">Süregelen husumetler</h4>
          <ul className="rel__rows">
            {conflicts.map((e, i) => (
              <li key={i} className="rel__row">
                <span
                  className="rel__dot"
                  style={{ background: (EDGE_KINDS[e.kind] ?? EDGE_KINDS.ticaret).color }}
                  aria-hidden="true"
                />
                <span className="rel__row-body">
                  <button type="button" className="dt__linkbtn rel__row-name" onClick={() => onFocus(e.from)}>
                    {nameOf(e.from)} → {nameOf(e.to)}
                  </button>
                  <span className="rel__row-label">{e.label}</span>
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      <h4 className="rel__subtitle">En güçlü ticaret bağları</h4>
      <ul className="rel__rows">
        {trades.map((e, i) => (
          <li key={i} className="rel__row">
            <span className="rel__dot" style={{ background: "var(--gain-dim)" }} aria-hidden="true" />
            <span className="rel__row-body">
              <button type="button" className="dt__linkbtn rel__row-name" onClick={() => onFocus(e.from)}>
                {nameOf(e.from)} ↔ {nameOf(e.to)}
              </button>
              <span className="rel__row-label">{e.label}</span>
            </span>
          </li>
        ))}
        {trades.length === 0 && <li className="dt__state">henüz ticaret bağı yok</li>}
      </ul>
    </>
  );
}
