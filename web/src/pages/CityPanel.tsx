import { useDetail } from "../hooks/useDetail";
import type { CityDetail } from "../types-detail";
import type { BucketHistory } from "../lib/derive";
import type { Snapshot } from "../types";
import { compact, lira2, tickLabel } from "../lib/format";
import { roleInk } from "../lib/roles";
import { Sparkline } from "../components/sparkline/Sparkline";
import { Block, DetailShell, RankList, Stat } from "./DetailShell";

/**
 * Şehir sayfası — "bu şehirde ne dönüyor" sorusunun tek ekranlık cevabı.
 *
 * Sıra bilinçli: önce şehrin büyüklüğü (kaç fabrika, kaç işçi), sonra
 * kimin elinde olduğu (aktörler, yoğunlaşma), sonra ne dönüyor (üretim,
 * hacim), en sonda tek tek işlemler. Genelden özele; göz yukarıdan aşağı
 * indikçe soru daralıyor.
 */

interface Props {
  slug: string;
  tick: number;
  snapshot: Snapshot | null;
  bucketHistory: BucketHistory;
  onClose: () => void;
  onSelectFirm: (id: number) => void;
}

export function CityPanel({
  slug,
  tick,
  snapshot,
  bucketHistory,
  onClose,
  onSelectFirm,
}: Props) {
  const { data, error, loading } = useDetail<CityDetail>(`/api/city/${slug}`, tick);

  // Bu şehrin fiyatları — hepsi tek bir eksene binmesin diye küçük çoklu.
  // Ürünlerin fiyat seviyeleri çok farklı (Zeytin ~16₺, Ziyafet ~300₺);
  // ortak eksende ucuz olanlar dibe yapışıp okunmaz oluyor. Her ürün kendi
  // ölçeğinde, güncel fiyatı yanında yazılı.
  const priceRows = (snapshot?.prices ?? [])
    .filter((c) => c.city === slug && c.last_lira != null)
    .map((c) => ({
      product: c.product,
      label: c.product_label,
      isRaw: c.is_raw,
      last: c.last_lira as number,
      baseline: c.baseline_lira,
      series: bucketHistory[`${c.city}/${c.product}`] ?? [],
    }))
    .filter((r) => r.series.length >= 2);

  const staffPct =
    data && data.required_employees > 0
      ? Math.round((data.employees / data.required_employees) * 100)
      : 0;

  return (
    <DetailShell
      eyebrow="Şehir"
      title={data?.label ?? slug}
      meta={
        data?.window_from_tick != null && (
          <span>
            istatistik penceresi {tickLabel(data.window_from_tick)}–{tickLabel(data.tick)}
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
            <Stat
              label="Fabrika"
              value={data.factory_count}
              sub={`${data.factory_count - data.idle_factory_count} çalışıyor`}
            />
            <Stat label="Özel çiftlik" value={data.farm_count} sub="pazara çıkmayan arz" />
            <Stat
              label="Kadro"
              value={`${data.employees}/${data.required_employees}`}
              sub={`tam kadronun %${staffPct}'i`}
            />
            <Stat
              label="Fabrika yoğunlaşması"
              value={data.factory_gini.toFixed(2)}
              sub={data.factory_gini < 0.3 ? "dağınık" : "tek elde toplanıyor"}
            />
            <Stat
              label="Stok yoğunlaşması"
              value={data.stock_gini.toFixed(2)}
              sub="0 eşit · 1 tek elde"
            />
          </div>

          <div className="dt__grid">
            {/* Kim kimle — kullanıcının açıkça istediği görünüm. */}
            <Block title="Kim kimle iş yapıyor" note="satıcı → alıcı">
              <RankList
                emptyText="bu şehirde henüz eşleşme yok"
                rows={data.top_pairs.map((p) => ({
                  key: `${p.seller.id}-${p.buyer.id}`,
                  label: (
                    <>
                      <span style={{ color: roleInk(p.seller.role) }}>{p.seller.name}</span>
                      <span className="dt__rank-arrow"> → </span>
                      <span style={{ color: roleInk(p.buyer.role) }}>{p.buyer.name}</span>
                    </>
                  ),
                  value: p.value_lira,
                  display: `${compact(p.value_lira)}₺`,
                }))}
              />
            </Block>

            <Block title="Hacim" note="dönen para">
              <RankList
                emptyText="işlem yok"
                rows={data.volume.slice(0, 8).map((v) => ({
                  key: v.product,
                  label: v.product_label,
                  value: v.value_lira,
                  display: `${compact(v.value_lira)}₺`,
                  // Ham madde ile mamul iki farklı iş: renk bunu ayırıyor.
                  color: v.is_raw ? "var(--role-ciftci)" : "var(--role-sanayici)",
                }))}
              />
              <p className="dt__legend">
                <span>
                  <i style={{ background: "var(--role-ciftci)" }} /> ham madde
                </span>
                <span>
                  <i style={{ background: "var(--role-sanayici)" }} /> mamul
                </span>
              </p>
            </Block>

            <Block title="Ne üretiliyor" note="fabrika · atıl">
              {data.production.length === 0 ? (
                <p className="dt__state">bu şehirde fabrika yok</p>
              ) : (
                <ul className="dt__ranks">
                  {data.production.map((p) => (
                    <li key={p.product} className="dt__rank">
                      <span className="dt__rank-label">{p.product_label}</span>
                      <span className="dt__rank-track" aria-hidden="true">
                        {/* Çalışan ve atıl payı yan yana; aralarında yüzey
                            boşluğu var ki iki parça birbirine karışmasın. */}
                        <span
                          className="dt__rank-fill"
                          style={{
                            width: `${((p.factories - p.idle) / p.factories) * 100}%`,
                            background: "var(--gain-dim)",
                          }}
                        />
                      </span>
                      <span className="dt__rank-value">
                        {p.factories - p.idle}/{p.factories}
                        <span className="dt__muted"> · {p.produced_units}br</span>
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </Block>

            {/* Fiyat seyri — eskiden yalnız /analytics altındaydı. */}
            <Block title="Fiyat seyri" note="her ürün kendi ölçeğinde" wide>
              {priceRows.length === 0 ? (
                <p className="dt__state">henüz fiyat oluşmadı</p>
              ) : (
                <>
                  <ul className="dt__smalls">
                    {priceRows.map((r) => (
                      <li key={r.product} className="dt__small">
                        <span className="dt__small-head">
                          <span className="dt__small-name">{r.label}</span>
                          <span
                            className={`dt__small-val${
                              r.last >= r.baseline ? " is-up" : " is-down"
                            }`}
                          >
                            {lira2(r.last)}₺
                          </span>
                        </span>
                        <span className="dt__small-chart">
                          <Sparkline values={r.series} baseline={r.baseline} />
                        </span>
                      </li>
                    ))}
                  </ul>
                  <p className="dt__legend">
                    <span>renk baz fiyata göre yön · yeşil üstünde, kırmızı altında</span>
                  </p>
                </>
              )}
            </Block>

            <Block title="Şehirde tutulan stok" note="değere göre">
              <RankList
                emptyText="stok yok"
                rows={data.stock.slice(0, 8).map((s) => ({
                  key: s.product,
                  label: `${s.product_label} · ${s.holders} elde`,
                  value: s.value_lira,
                  display: `${compact(s.units)}br`,
                  color: s.is_raw ? "var(--role-ciftci)" : "var(--role-sanayici)",
                }))}
              />
            </Block>

            {/* Aktörler — firmaya geçiş noktası. */}
            <Block title="Şehirdeki firmalar" note="fabrika · stok · alım/satım" wide>
              <div className="dt__scroll">
                <table className="dt__table">
                  <thead>
                    <tr>
                      <th>firma</th>
                      <th>rol</th>
                      <th className="num">fabrika</th>
                      <th className="num">çiftlik</th>
                      <th className="num">stok</th>
                      <th className="num">stok ₺</th>
                      <th className="num">aldı</th>
                      <th className="num">sattı</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.actors.slice(0, 24).map((a) => (
                      <tr
                        key={a.actor.id}
                        className="is-clickable"
                        onClick={() => onSelectFirm(a.actor.id)}
                      >
                        <td className="name">
                          <button
                            type="button"
                            className="dt__linkbtn"
                            onClick={(e) => {
                              e.stopPropagation();
                              onSelectFirm(a.actor.id);
                            }}
                          >
                            {a.actor.name}
                          </button>
                        </td>
                        <td style={{ color: roleInk(a.actor.role) }}>{a.actor.role ?? "—"}</td>
                        <td className="num">{a.factories}</td>
                        <td className="num">{a.farms}</td>
                        <td className="num">{a.stock_units}</td>
                        <td className="num">{compact(a.stock_value_lira)}</td>
                        <td className="num">{a.bought_units}</td>
                        <td className="num">{a.sold_units}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Block>

            <Block title="Son işlemler" note={`son ${data.recent_trades.length}`} wide>
              <div className="dt__scroll">
                <table className="dt__table">
                  <thead>
                    <tr>
                      <th>tick</th>
                      <th>ürün</th>
                      <th className="num">adet</th>
                      <th className="num">fiyat</th>
                      <th>satıcı</th>
                      <th>alıcı</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.recent_trades.map((t, i) => (
                      <tr key={`${t.tick}-${i}`}>
                        <td>{tickLabel(t.tick)}</td>
                        <td>{t.product_label}</td>
                        <td className="num">{t.quantity}</td>
                        <td className="num">{lira2(t.price_lira)}₺</td>
                        <td className="name" style={{ color: roleInk(t.seller.role) }}>
                          {t.seller.name}
                        </td>
                        <td className="name" style={{ color: roleInk(t.buyer.role) }}>
                          {t.buyer.name}
                        </td>
                      </tr>
                    ))}
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
