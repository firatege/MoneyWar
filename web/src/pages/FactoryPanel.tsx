import { useDetail } from "../hooks/useDetail";
import type { FactoryDetail } from "../types-detail";
import { compact, lira2, tickLabel } from "../lib/format";
import { roleInk } from "../lib/roles";
import { Block, DetailShell, Stat } from "./DetailShell";

/**
 * Fabrika sayfası — tek soruya odaklı: **bu bant neden dönüyor / dönmüyor?**
 *
 * Girdi tablosu bunun için var. Motor tam batch yoksa üretimi durdurmaz,
 * batch'i `batch/4`'e kadar küçültür; yani "eksik" ile "durdu" farklı
 * şeyler. Her satır bu yüzden iki eşiği birden gösteriyor: tam batch
 * ihtiyacı (çubuğun sonu) ve bandın hiç dönemeyeceği alt sınır (dikey
 * işaret). Stok işaretin solundaysa bant duruyor, arasındaysa yavaşlıyor.
 */

interface Props {
  id: number;
  tick: number;
  onClose: () => void;
  onSelectFirm: (id: number) => void;
}

export function FactoryPanel({ id, tick, onClose, onSelectFirm }: Props) {
  const { data, error, loading } = useDetail<FactoryDetail>(`/api/factory/${id}`, tick);

  const blocking = data?.inputs.filter((i) => i.blocking) ?? [];
  const history = data?.production_history.slice(-40) ?? [];
  const maxUnits = Math.max(1, ...history.map((h) => h.units));

  return (
    <DetailShell
      eyebrow="Fabrika"
      title={data ? `${data.product_label} · ${data.city_label}` : `#${id}`}
      meta={
        data && (
          <>
            <span>sv{data.level}</span>
            <button
              type="button"
              className="dt__linkbtn"
              style={{ color: roleInk(data.owner.role) }}
              onClick={() => onSelectFirm(data.owner.id)}
            >
              {data.owner.name} →
            </button>
          </>
        )
      }
      loading={loading}
      error={error}
      onClose={onClose}
    >
      {data && (
        <>
          <div className="dt__stats">
            <Stat
              label="Durum"
              value={data.idle ? "atıl" : "çalışıyor"}
              tone={data.idle ? "loss" : "gain"}
              sub={
                data.ticks_since_production == null
                  ? "hiç üretmedi"
                  : `son üretim ${data.ticks_since_production} tick önce`
              }
            />
            <Stat
              label="Kadro"
              value={`${data.employees}/${data.required_employees}`}
              sub={`üretim ×${data.staffing.toFixed(2)}`}
            />
            <Stat
              label="Üretti"
              value={`${compact(data.produced_units)} br`}
              sub={`bekleyen ${data.pending_units} br`}
            />
            <Stat label="Depoda" value={`${data.output_stock} br`} sub="satılmayı bekliyor" />
            <Stat
              label="Marj"
              value={data.margin_pct == null ? "—" : `%${data.margin_pct.toFixed(0)}`}
              tone={data.margin_pct != null && data.margin_pct < 0 ? "loss" : "gain"}
              sub={
                data.unit_cost_lira == null
                  ? "maliyet bilinmiyor"
                  : `${lira2(data.unit_cost_lira)}₺ → ${lira2(data.market_price_lira)}₺`
              }
            />
          </div>

          {/* Kadro ölçeri — üretim buna orantılı. Ölçer tek başına sayfayı
              bölen bir çizgi gibi duruyordu; değeriyle birlikte ve dar. */}
          <Block title="Kadro doluluğu" note="üretim bu oranla ölçeklenir">
            <div className="dt__meter-row">
              <div
                className="dt__meter"
                role="meter"
                aria-valuenow={data.employees}
                aria-valuemin={0}
                aria-valuemax={data.required_employees || 1}
                aria-label="Çalışan / tam kadro"
              >
                <span className="dt__meter-fill" style={{ width: `${data.staffing * 100}%` }} />
              </div>
              <span className="dt__meter-val">
                {data.employees}/{data.required_employees}
                <span className="dt__muted"> · %{Math.round(data.staffing * 100)}</span>
              </span>
            </div>
          </Block>

          <div className="dt__grid">
            {/* Tanı: bant neden duruyor. */}
            <Block
              title="Girdi durumu"
              note={blocking.length > 0 ? `${blocking.length} girdi bandı durduruyor` : "girdiler yeterli"}
              wide
            >
              <ul className="dt__inputs">
                {data.inputs.map((i) => {
                  // Ölçek tam batch ihtiyacı; stok bunu aşabilir, çubuk taşmasın.
                  const scale = Math.max(i.required, i.available, 1);
                  return (
                    <li key={i.product} className="dt__input">
                      <span className="dt__rank-label">
                        {i.product_label}
                        {i.is_primary && <span className="dt__muted"> · ana</span>}
                      </span>
                      <span className="dt__input-track" aria-hidden="true">
                        <span
                          className={`dt__input-have${i.blocking ? " is-short" : ""}`}
                          style={{ width: `${(i.available / scale) * 100}%` }}
                        />
                        {/* Bandın duracağı eşik. */}
                        <span
                          className="dt__input-min"
                          style={{ left: `${(i.min_required / scale) * 100}%` }}
                          title="bu çizginin solunda bant durur"
                        />
                      </span>
                      <span className="dt__input-num">
                        {i.available} / {i.required}
                        {i.blocking && <span className="dt__flag">DURDU</span>}
                        {i.partial && <span className="dt__flag dt__flag--partial">KISMİ</span>}
                      </span>
                    </li>
                  );
                })}
              </ul>
              <p className="dt__legend">
                <span>stok / tam batch ihtiyacı · dikey çizgi: bandın alt sınırı</span>
              </p>
            </Block>

            {/* Üretim geçmişi — adım grafiği (tek seri, sayılabilir). */}
            <Block title="Üretim geçmişi" note={`son ${history.length} batch`}>
              {history.length === 0 ? (
                <p className="dt__state">bu pencerede üretim yok</p>
              ) : (
                <>
                  <div className="dt__steps" aria-hidden="true">
                    {history.map((h, i) => (
                      <span
                        key={`${h.tick}-${i}`}
                        className="dt__step"
                        style={{ height: `${(h.units / maxUnits) * 100}%` }}
                        title={`${tickLabel(h.tick)} · ${h.units} br`}
                      />
                    ))}
                  </div>
                  <p className="dt__legend">
                    <span>
                      {tickLabel(history[0].tick)} → {tickLabel(history[history.length - 1].tick)} ·
                      en büyük batch {maxUnits} br
                    </span>
                  </p>
                </>
              )}
            </Block>

            <Block title="İşlenen batch'ler" note="tamamlanmayı bekleyen">
              {data.batches.length === 0 ? (
                <p className="dt__state">bantta batch yok</p>
              ) : (
                <ul className="dt__ranks">
                  {data.batches.map((b, i) => (
                    <li key={i} className="dt__rank">
                      <span className="dt__rank-label">{b.units} br</span>
                      <span className="dt__rank-track" aria-hidden="true">
                        <span
                          className="dt__rank-fill"
                          style={{
                            width: `${
                              100 -
                              (b.ticks_remaining /
                                Math.max(1, b.completion_tick - b.started_tick)) *
                                100
                            }%`,
                            background: "var(--role-sanayici)",
                          }}
                        />
                      </span>
                      <span className="dt__rank-value">{b.ticks_remaining} tick</span>
                    </li>
                  ))}
                </ul>
              )}
            </Block>

            <Block title="Bu üründen son satışlar" note="sahibin bu şehirdeki satışı" wide>
              {data.recent_sales.length === 0 ? (
                <p className="dt__state">bu pencerede satış yok</p>
              ) : (
                <div className="dt__scroll">
                  <table className="dt__table">
                    <thead>
                      <tr>
                        <th>tick</th>
                        <th className="num">adet</th>
                        <th className="num">fiyat</th>
                        <th className="num">tutar</th>
                        <th>alıcı</th>
                      </tr>
                    </thead>
                    <tbody>
                      {data.recent_sales.map((t, i) => (
                        <tr key={`${t.tick}-${i}`}>
                          <td>{tickLabel(t.tick)}</td>
                          <td className="num">{t.quantity}</td>
                          <td className="num">{lira2(t.price_lira)}₺</td>
                          <td className="num">{compact(t.value_lira)}₺</td>
                          <td className="name">
                            <button
                              type="button"
                              className="dt__linkbtn"
                              style={{ color: roleInk(t.buyer.role) }}
                              onClick={() => onSelectFirm(t.buyer.id)}
                            >
                              {t.buyer.name}
                            </button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </Block>
          </div>
        </>
      )}
    </DetailShell>
  );
}
