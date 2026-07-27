import { useDetail } from "../hooks/useDetail";
import type { FirmDetail } from "../types-detail";
import type { PnlPoint } from "../lib/derive";
import { Sparkline } from "../components/sparkline/Sparkline";
import { compact, lira2, signedCompact, tickLabel } from "../lib/format";
import { roleInk } from "../lib/roles";
import { Block, DetailShell, RankList, Stat } from "./DetailShell";

/**
 * Firma sayfası — "bu firma ne yapıyor, iyi mi gidiyor" sorusu.
 *
 * Alım/satım kutusu iki kez elden geçti ve ikisi de veriyle çürüdü:
 *
 *   1. Halter (dumbbell), iki fiyat ortak eksende. Ürünlerin fiyat
 *      seviyeleri çok uzak (Zeytin ~16₺, Zeytinyağı ~112₺); ortak eksende
 *      ucuz ürünün makası sıfıra eziliyordu.
 *   2. Ürün başına marj yüzdesi. Sanayici'de satırların **hepsi** boş çıktı —
 *      çünkü üretici bir ürünü alıp aynı ürünü satmaz: Zeytin alır,
 *      Zeytinyağı satar. Ürün başına marj üretici için tanımsız bir soru.
 *
 * Şimdiki biçim iki yönlü çubuk: sıfır ortada, sol aldığı, sağ sattığı.
 * Üreticide desen kendini gösteriyor (girdiler solda, mamul sağda);
 * tüccarda aynı ürün iki yana birden düşer ve marj orada gerçekten
 * anlamlıdır — o yüzden yüzde yalnız iki yön de varsa yazılıyor.
 */

interface Props {
  id: number;
  tick: number;
  /** Bu sezonun PnL zaman serisi (istemcide birikiyor). */
  pnlHistory: PnlPoint[];
  onClose: () => void;
  onSelectFactory: (id: number) => void;
  onSelectFirm: (id: number) => void;
}

export function FirmPanel({
  id,
  tick,
  pnlHistory,
  onClose,
  onSelectFactory,
  onSelectFirm,
}: Props) {
  const { data, error, loading } = useDetail<FirmDetail>(`/api/firm/${id}`, tick);

  // İki yönlü çubuğun ortak ölçeği: tek bir yönde dönen en büyük tutar.
  // Aynı ölçek iki yana da uygulanıyor ki sol ve sağ karşılaştırılabilsin.
  const flow = (data?.flow ?? []).filter((f) => f.bought_units > 0 || f.sold_units > 0);
  const flowScale = Math.max(
    1,
    ...flow.map((f) => Math.max(f.buy_value_lira, f.sell_value_lira)),
  );
  // Pazar kontrolü: gürültüyü kes — payı %1'in altında olan pazar
  // "kontrol" değil, tek seferlik satış.
  const grip = (data?.market_grip ?? []).filter((g) => g.share_pct >= 1).slice(0, 12);
  const monopolies = grip.filter((g) => g.is_monopolist).length;

  /** Marj yalnız iki yön de varsa anlamlı (tüccar arbitrajı). */
  const marginOf = (f: { avg_buy_lira: number | null; avg_sell_lira: number | null }) =>
    f.avg_buy_lira != null && f.avg_sell_lira != null && f.avg_buy_lira > 0
      ? ((f.avg_sell_lira - f.avg_buy_lira) / f.avg_buy_lira) * 100
      : null;

  return (
    <DetailShell
      eyebrow={data?.actor.role ?? "Firma"}
      title={data?.actor.name ?? `#${id}`}
      meta={
        data && (
          <>
            {data.rank != null && <span>sıra {data.rank}</span>}
            {data.window_from_tick != null && (
              <span>
                {tickLabel(data.window_from_tick)}–{tickLabel(data.tick)}
              </span>
            )}
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
            <Stat label="Nakit" value={`${compact(data.cash_lira)}₺`} />
            <Stat label="Stok değeri" value={`${compact(data.stock_value_lira)}₺`} />
            <Stat
              label="Kâr / zarar"
              value={`${signedCompact(data.pnl_lira)}₺`}
              tone={data.pnl_lira < 0 ? "loss" : "gain"}
              sub="sezon başına göre"
            />
            <Stat label="Fabrika" value={data.factories.length} sub={`${data.farms.length} çiftlik`} />
          </div>

          {/* PnL eğrisi — eskiden yalnız /analytics/firm/:id altındaydı.
              Tek sayı "şu an ne durumda" der; eğri "nasıl geldi" der ve
              izleyicinin asıl merak ettiği bu. */}
          {pnlHistory.length >= 2 && (
            <Block title="Kâr / zarar seyri" note={`${pnlHistory.length} tick`} wide>
              <div className="dt__pnl">
                <Sparkline
                  values={pnlHistory.map((p) => p.pnl)}
                  width={600}
                  height={72}
                  baseline={0}
                />
              </div>
              <p className="dt__legend">
                <span>
                  {tickLabel(pnlHistory[0].tick)} → {tickLabel(pnlHistory[pnlHistory.length - 1].tick)}
                  {" · "}sıfır çizgisi başa baş
                </span>
              </p>
            </Block>
          )}

          {/* Pazar kontrolü — "nerede güçlüyüm" sorusu.
              Fabrika listesi nerede *ürettiğini* söyler; bu, pazarı kimin
              **tuttuğunu**. İkisi aynı şey değil: tesisi olmadığın pazarda
              da satıyor olabilirsin (alıp sattığın mal). */}
          {grip.length > 0 && (
            <Block
              title="Pazar kontrolü"
              note={`${monopolies} tekel · ${grip.length} pazarda satıyor`}
              wide
            >
              <ul className="dt__grip">
                {grip.map((g) => (
                  <li
                    key={`${g.city}-${g.product}`}
                    className={g.is_monopolist ? "dt__grip-row is-mono" : "dt__grip-row"}
                  >
                    <span className="dt__grip-name">
                      {g.product_label}
                      <span className="dt__grip-city">{g.city_label}</span>
                    </span>
                    {/* Çubuk: dolu kısım firmanın payı, çentik en yakın
                        rakibin payı. Tekel olup olmadığı çentiğin nerede
                        durduğundan okunuyor — tek sayı yerine yarış. */}
                    <span className="dt__grip-bar" aria-hidden="true">
                      <span className="dt__grip-fill" style={{ width: `${g.share_pct}%` }} />
                      {g.runner_up_pct > 0 && (
                        <span
                          className="dt__grip-rival"
                          style={{ left: `${Math.min(g.runner_up_pct, 100)}%` }}
                        />
                      )}
                    </span>
                    <span className="dt__grip-pct">
                      %{g.share_pct}
                      {g.is_monopolist && <span className="dt__grip-flag">tekel</span>}
                    </span>
                    <span className="dt__grip-meta">
                      {g.units} br
                      {g.factories > 0 && ` · ${g.factories} tesis`}
                    </span>
                  </li>
                ))}
              </ul>
              <p className="dt__note">
                Dolu çubuk firmanın payı, ince çizgi en yakın rakibin payı.
                Rakip çizgisi ne kadar geride, tutuş o kadar sağlam.
              </p>
            </Block>
          )}

          {/* Fabrikalar — bir sonraki kademeye geçiş. */}
          <Block title="Fabrikaları" note="tıkla → fabrika sayfası" wide>
            {data.factories.length === 0 ? (
              <p className="dt__state">bu firmanın fabrikası yok</p>
            ) : (
              <ul className="dt__cards">
                {data.factories.map((f) => (
                  <li key={f.id}>
                    <button
                      type="button"
                      className={`dt__card${f.idle ? " dt__card--idle" : ""}`}
                      onClick={() => onSelectFactory(f.id)}
                    >
                      <span className="dt__card-top">
                        <span className="dt__card-name">{f.product_label}</span>
                        <span className="dt__card-tag">sv{f.level}</span>
                      </span>
                      <p className="dt__card-line">{f.city_label}</p>
                      <p className="dt__card-line">
                        kadro {f.employees}/{f.required_employees} ·{" "}
                        {f.idle ? <span className="is-warn">atıl</span> : "çalışıyor"}
                      </p>
                      <p className="dt__card-line">üretti {f.produced_units} br</p>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Block>

          <div className="dt__grid">
            {/* Ticaret ortakları + güven. */}
            <Block title="Ticaret ortakları" note="işlem hacmine göre">
              {data.partners.length === 0 ? (
                <p className="dt__state">henüz işlem yok</p>
              ) : (
                <ul className="dt__ranks">
                  {data.partners.map((p) => (
                    <li key={p.actor.id} className="dt__rank">
                      <span className="dt__rank-label">
                        <button
                          type="button"
                          className="dt__linkbtn"
                          style={{ color: roleInk(p.actor.role) }}
                          onClick={() => onSelectFirm(p.actor.id)}
                        >
                          {p.actor.name}
                        </button>
                      </span>
                      <span className="dt__rank-track" aria-hidden="true">
                        <span
                          className="dt__rank-fill"
                          style={{
                            width: `${
                              (p.value_lira /
                                Math.max(...data.partners.map((x) => x.value_lira), 1)) *
                              100
                            }%`,
                            background: "var(--accent-dim)",
                          }}
                        />
                      </span>
                      <span className="dt__rank-value">
                        {compact(p.value_lira)}₺
                        <span className="dt__muted"> güven {p.trust_score.toFixed(2)}</span>
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </Block>

            {/* Alım/satım akışı — sıfır ortada, sol aldığı, sağ sattığı. */}
            <Block title="Alım / satım akışı" note="sol aldığı · sağ sattığı">
              {flow.length === 0 ? (
                <p className="dt__state">henüz işlem yok</p>
              ) : (
                <>
                  <ul className="dt__gaps">
                    {flow.slice(0, 8).map((f) => {
                      const m = marginOf(f);
                      const buyPct = (f.buy_value_lira / flowScale) * 50;
                      const sellPct = (f.sell_value_lira / flowScale) * 50;
                      return (
                        <li key={f.product} className="dt__gap">
                          <span className="dt__rank-label">{f.product_label}</span>
                          <span className="dt__gap-track" aria-hidden="true">
                            <span className="dt__gap-zero" />
                            {f.buy_value_lira > 0 && (
                              <span
                                className="dt__gap-fill is-buy"
                                style={{ right: "50%", width: `${buyPct}%` }}
                              />
                            )}
                            {f.sell_value_lira > 0 && (
                              <span
                                className="dt__gap-fill is-sell"
                                style={{ left: "50%", width: `${sellPct}%` }}
                              />
                            )}
                          </span>
                          <span className="dt__gap-val">
                            {m != null ? (
                              <span className={m < 0 ? "is-neg" : "is-pos"}>
                                marj {m > 0 ? "+" : "−"}%{Math.abs(m).toFixed(0)}
                              </span>
                            ) : (
                              <span className="dt__muted">
                                {f.sold_units > 0
                                  ? `${compact(f.sell_value_lira)}₺ sattı`
                                  : `${compact(f.buy_value_lira)}₺ aldı`}
                              </span>
                            )}
                          </span>
                        </li>
                      );
                    })}
                  </ul>
                  <p className="dt__legend">
                    <span>
                      <i style={{ background: "var(--role-sanayici)" }} /> aldığı
                    </span>
                    <span>
                      <i style={{ background: "var(--role-tuccar)" }} /> sattığı
                    </span>
                    <span className="dt__muted">marj: aynı üründe iki yön varsa</span>
                  </p>
                </>
              )}
            </Block>

            <Block title="Envanteri" note="değere göre">
              <RankList
                emptyText="stok yok"
                rows={data.stock.slice(0, 10).map((s) => ({
                  key: `${s.city}-${s.product}`,
                  label: `${s.product_label} · ${s.city}`,
                  value: s.value_lira,
                  display: `${s.units}br`,
                }))}
              />
            </Block>

            <Block title="Özel çiftlikleri" note="pazara çıkmayan arz">
              {data.farms.length === 0 ? (
                <p className="dt__state">çiftliği yok</p>
              ) : (
                <RankList
                  rows={data.farms.map((f) => ({
                    key: String(f.id),
                    label: `${f.product_label} · ${f.city}`,
                    value: f.output_per_tick,
                    display: `${f.output_per_tick} br/tick · sv${f.level}`,
                    color: "var(--role-ciftci)",
                  }))}
                />
              )}
            </Block>

            <Block title="Son işlemleri" note={`son ${data.recent_trades.length}`} wide>
              <div className="dt__scroll">
                <table className="dt__table">
                  <thead>
                    <tr>
                      <th>tick</th>
                      <th>yön</th>
                      <th>ürün</th>
                      <th>şehir</th>
                      <th className="num">adet</th>
                      <th className="num">fiyat</th>
                      <th>karşı taraf</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.recent_trades.map((t, i) => {
                      const bought = t.buyer.id === data.actor.id;
                      const other = bought ? t.seller : t.buyer;
                      return (
                        <tr key={`${t.tick}-${i}`}>
                          <td>{tickLabel(t.tick)}</td>
                          <td style={{ color: bought ? "var(--role-sanayici)" : "var(--role-tuccar)" }}>
                            {bought ? "aldı" : "sattı"}
                          </td>
                          <td>{t.product_label}</td>
                          <td>{t.city}</td>
                          <td className="num">{t.quantity}</td>
                          <td className="num">{lira2(t.price_lira)}₺</td>
                          <td className="name">
                            <button
                              type="button"
                              className="dt__linkbtn"
                              style={{ color: roleInk(other.role) }}
                              onClick={() => onSelectFirm(other.id)}
                            >
                              {other.name}
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </Block>
          </div>
        </>
      )}
    </DetailShell>
  );
}
