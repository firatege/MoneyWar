import { useMemo, useState } from "react";
import { useDetail } from "../hooks/useDetail";
import type { RelationsGraph } from "../types-detail";
import { compact, tickLabel } from "../lib/format";
import { roleColor, roleInk } from "../lib/roles";
import { Block, DetailShell, Stat } from "./DetailShell";
import "./relations.css";

/**
 * İlişki sayfası — kim kiminle çalışıyor, kim kime düşman.
 *
 * # Çember graf denendi ve okunmadı
 *
 * İlk sürüm klasik bir chord/çember grafiydi: 36 firma çember üzerinde,
 * aralarındaki 226 bağ merkeze kavislenen yaylar. Teoride doğru, pratikte
 * hiçbir şey seçilmiyordu — yaylar ortada yumak oluyor, isimler radyal
 * döndüğü için okunmuyor, iki firmanın bağlı olup olmadığı ancak tıklayıp
 * filtreleyerek anlaşılıyordu.
 *
 * # Yerine ne kondu
 *
 * Sayfa okunur katmanlara ayrıldı ve **hiçbirinde çapraz çizgi yok**:
 *
 *   1. **Ekonominin akışı** — rolden role. Firma-firma çifti gürültü
 *      (226 kenar); role indirgeyince on satır kalıyor ve yapı görünüyor:
 *      Çiftçi ham çıkarır, Tüccar taşır, Sanayici işler, Alıcı tüketir.
 *   2. **Husumetler** — her çekişme bir kart: kim, kime, ne, ne zamandan beri.
 *   3. **En güçlü bağlar** — sıralı çubuk listesi, tıklanır.
 *
 * Firma seçilince en üstte o firmanın bütün ilişkileri tek tabloda açılıyor.
 */

/** Kenar türlerinin görsel kimliği ve okunur adı. */
const EDGE_KINDS: Record<string, { label: string; color: string }> = {
  ticaret: { label: "ticaret", color: "var(--gain-dim)" },
  kin: { label: "kin", color: "var(--role-spekulator)" },
  savas: { label: "fiyat savaşı", color: "var(--loss)" },
  bogma: { label: "tedarik boğma", color: "var(--role-alici)" },
};

/**
 * Rolün üretim zincirindeki yeri. Akış oku bunu izler: zincirin yönünde
 * giden akış dolu, geri dönen soluk çizilir.
 */
const ROLE_STAGE: Record<string, number> = {
  Çiftçi: 0,
  Spekülatör: 1,
  Tüccar: 1,
  Sanayici: 2,
  Alıcı: 3,
  Banka: 4,
};

interface Props {
  tick: number;
  onClose: () => void;
  onSelectFirm: (id: number) => void;
}

export function RelationsPanel({ tick, onClose, onSelectFirm }: Props) {
  const { data, error, loading } = useDetail<RelationsGraph>("/api/relations", tick);
  const [focused, setFocused] = useState<number | null>(null);

  const nameOf = (id: number) => data?.nodes.find((n) => n.id === id)?.name ?? `#${id}`;
  const roleOf = (id: number) => data?.nodes.find((n) => n.id === id)?.role ?? null;

  const conflicts = useMemo(
    () => (data?.edges ?? []).filter((e) => e.kind !== "ticaret"),
    [data],
  );

  const topTrades = useMemo(
    () =>
      [...(data?.edges ?? []).filter((e) => e.kind === "ticaret")]
        .sort((a, b) => (b.value_lira ?? 0) - (a.value_lira ?? 0))
        .slice(0, 12),
    [data],
  );

  const focusedRows = useMemo(() => {
    if (focused == null || !data) return [];
    return data.edges
      .filter((e) => e.from === focused || e.to === focused)
      .map((e) => ({ edge: e, other: e.from === focused ? e.to : e.from }))
      .sort((a, b) => {
        // Husumet üstte — az ve önemli.
        if (a.edge.kind === b.edge.kind) return (b.edge.value_lira ?? 0) - (a.edge.value_lira ?? 0);
        return a.edge.kind === "ticaret" ? 1 : -1;
      });
  }, [data, focused]);

  const focusedNode = data?.nodes.find((n) => n.id === focused) ?? null;
  const maxFlow = Math.max(1, ...(data?.role_flows ?? []).map((f) => f.value_lira));
  const maxTrade = Math.max(1, ...topTrades.map((t) => t.value_lira ?? 0));

  return (
    <DetailShell
      eyebrow="İlişkiler"
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

          {/* Seçili firmanın bütün ilişkileri — en üstte, tek tabloda. */}
          {focusedNode && (
            <Block
              title={focusedNode.name}
              note={`${focusedNode.partners} ortak · ${focusedNode.rivals} husumet`}
              wide
            >
              <p className="rel__who">
                <span style={{ color: roleInk(focusedNode.role) }}>{focusedNode.role ?? "—"}</span>
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
                <button type="button" className="rel__clear" onClick={() => setFocused(null)}>
                  ← herkes
                </button>
              </p>
              <div className="dt__scroll">
                <table className="dt__table">
                  <thead>
                    <tr>
                      <th>ilişki</th>
                      <th>karşı taraf</th>
                      <th>ne oluyor</th>
                      <th className="num">hacim</th>
                      <th className="num">güven</th>
                    </tr>
                  </thead>
                  <tbody>
                    {focusedRows.map(({ edge, other }, i) => {
                      const st = EDGE_KINDS[edge.kind] ?? EDGE_KINDS.ticaret;
                      return (
                        <tr key={`${edge.kind}-${other}-${i}`}>
                          <td>
                            <span
                              className="rel__tag"
                              style={{ borderColor: st.color, color: st.color }}
                            >
                              {st.label}
                            </span>
                          </td>
                          <td className="name">
                            <button
                              type="button"
                              className="dt__linkbtn"
                              style={{ color: roleInk(roleOf(other)) }}
                              onClick={() => setFocused(other)}
                            >
                              {nameOf(other)}
                            </button>
                          </td>
                          <td>{edge.label}</td>
                          <td className="num">
                            {edge.value_lira != null ? `${compact(edge.value_lira)}₺` : "—"}
                          </td>
                          <td className="num">{edge.trust != null ? edge.trust.toFixed(2) : "—"}</td>
                        </tr>
                      );
                    })}
                    {focusedRows.length === 0 && (
                      <tr>
                        <td colSpan={5}>bu firmanın kayıtlı ilişkisi yok</td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </Block>
          )}

          {/* ── 1. Ekonominin akışı ─────────────────────────────────────── */}
          <Block title="Ekonominin akışı" note="rolden role · dönen para" wide>
            <p className="rel__intro">
              Firma-firma bağı çok ve gürültülü. Role indirgeyince ekonominin şekli
              görünüyor: <b>Çiftçi</b> ham madde çıkarır, <b>Tüccar</b> şehirler arası
              taşır, <b>Sanayici</b> işler, <b>Alıcı</b> tüketir. <b>Spekülatör</b> araya
              girip stok tutar. Dolu çubuk zincirin yönünde, soluk olan geri akış.
            </p>
            <ul className="rel__flows">
              {data.role_flows.map((f) => {
                const forward = (ROLE_STAGE[f.to_role] ?? 9) >= (ROLE_STAGE[f.from_role] ?? 9);
                return (
                  <li key={`${f.from_role}-${f.to_role}`} className="rel__flow">
                    <span className="rel__flow-pair">
                      <span style={{ color: roleInk(f.from_role) }}>{f.from_role}</span>
                      <span className="rel__flow-arrow" aria-hidden="true">
                        {forward ? "→" : "↩"}
                      </span>
                      <span style={{ color: roleInk(f.to_role) }}>{f.to_role}</span>
                    </span>
                    <span className="rel__flow-track" aria-hidden="true">
                      <span
                        className={`rel__flow-fill${forward ? "" : " is-back"}`}
                        style={{
                          width: `${(f.value_lira / maxFlow) * 100}%`,
                          background: roleColor(f.from_role),
                        }}
                      />
                    </span>
                    <span className="rel__flow-val">{compact(f.value_lira)}₺</span>
                    <span className="rel__flow-what">{f.top_products.join(" · ")}</span>
                  </li>
                );
              })}
              {data.role_flows.length === 0 && <li className="dt__state">henüz akış yok</li>}
            </ul>
          </Block>

          <div className="dt__grid">
            {/* ── 2. Husumetler ────────────────────────────────────────── */}
            <Block title="Husumetler" note={`${conflicts.length} aktif`}>
              {conflicts.length === 0 ? (
                <p className="dt__state">piyasa sakin — kimse kimseye kin tutmuyor</p>
              ) : (
                <ul className="rel__cards">
                  {conflicts.map((e, i) => {
                    const st = EDGE_KINDS[e.kind] ?? EDGE_KINDS.ticaret;
                    return (
                      <li key={i} className="rel__card" style={{ borderLeftColor: st.color }}>
                        <span className="rel__card-kind" style={{ color: st.color }}>
                          {st.label}
                        </span>
                        <p className="rel__card-who">
                          <button
                            type="button"
                            className="dt__linkbtn"
                            style={{ color: roleInk(roleOf(e.from)) }}
                            onClick={() => setFocused(e.from)}
                          >
                            {nameOf(e.from)}
                          </button>
                          <span className="dt__muted"> → </span>
                          <button
                            type="button"
                            className="dt__linkbtn"
                            style={{ color: roleInk(roleOf(e.to)) }}
                            onClick={() => setFocused(e.to)}
                          >
                            {nameOf(e.to)}
                          </button>
                        </p>
                        <p className="rel__card-what">{e.label}</p>
                      </li>
                    );
                  })}
                </ul>
              )}
              <p className="rel__note">
                Husumet kendiliğinden doğuyor: fiyatı kırılan firma <b>kin</b> tutuyor,
                kin tutan sonraki kararlarında sert davranıyor. Yani olay ilişkiyi,
                ilişki de olayı besliyor.
              </p>
            </Block>

            {/* ── 3. En güçlü bağlar ───────────────────────────────────── */}
            <Block title="En güçlü ticaret bağları" note="dönen paraya göre">
              <ul className="rel__bonds">
                {topTrades.map((e, i) => (
                  <li key={i} className="rel__bond">
                    <span className="rel__bond-pair">
                      <button
                        type="button"
                        className="dt__linkbtn"
                        style={{ color: roleInk(roleOf(e.from)) }}
                        onClick={() => setFocused(e.from)}
                      >
                        {nameOf(e.from)}
                      </button>
                      <span className="dt__muted"> ↔ </span>
                      <button
                        type="button"
                        className="dt__linkbtn"
                        style={{ color: roleInk(roleOf(e.to)) }}
                        onClick={() => setFocused(e.to)}
                      >
                        {nameOf(e.to)}
                      </button>
                    </span>
                    <span className="rel__bond-track" aria-hidden="true">
                      <span
                        className="rel__bond-fill"
                        style={{ width: `${((e.value_lira ?? 0) / maxTrade) * 100}%` }}
                      />
                    </span>
                    <span className="rel__bond-val">
                      {compact(e.value_lira ?? 0)}₺
                      <span className="dt__muted"> güven {(e.trust ?? 0).toFixed(2)}</span>
                    </span>
                  </li>
                ))}
                {topTrades.length === 0 && <li className="dt__state">henüz ticaret bağı yok</li>}
              </ul>
              <p className="rel__note">
                Güven işlem sayısıyla birikir ve teklifi yükseltir — tanıdık satıcıya
                daha fazla ödenir. Firma adına tıkla, bütün ilişkilerini gör.
              </p>
            </Block>
          </div>
        </>
      )}
    </DetailShell>
  );
}
