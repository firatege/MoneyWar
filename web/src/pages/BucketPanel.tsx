import { useDetail } from "../hooks/useDetail";
import type { BucketDetail } from "../types-detail";
import type { BucketHistory } from "../lib/derive";
import { lira2, tickLabel } from "../lib/format";
import { Sparkline } from "../components/sparkline/Sparkline";
import { Block, DetailShell, Stat } from "./DetailShell";

/**
 * Ürün sayfası — "bu fiyat normal mi?" sorusunun cevabı.
 *
 * Izgaradaki hücre tek başına yalnız anlık fiyatı ve küçük bir sparkline'ı
 * gösteriyordu; ikisi de "26₺ pahalı mı ucuz mu" sorusunu cevaplamıyordu.
 * Burada üç referans yan yana duruyor: sezon başına göre nerede, beş şehir
 * ortalamasına göre nerede, ve seyri boyunca hangi aralıkta gezmiş.
 *
 * Sıra genelden özele: önce ürünün kendisi (zincirdeki yeri, pazar
 * ortalaması), sonra şehir kırılımı — çünkü arbitraj fırsatı ancak
 * ortalamayı bilince görünür.
 */

interface Props {
  city: string;
  product: string;
  tick: number;
  bucketHistory: BucketHistory;
  onClose: () => void;
  onSelectCity: (slug: string) => void;
}

/** Yüzdeyi işaretiyle ve okunur basamakla yazar. */
function pct(v: number): string {
  const abs = Math.abs(v);
  const digits = abs >= 100 ? 0 : 1;
  return `${v > 0 ? "+" : v < 0 ? "−" : ""}${abs.toFixed(digits)}%`;
}

/** Yukarı payına göre okunur yorum — 50 civarı dengeli salınım. */
function swingLabel(upShare: number, moves: number): string {
  if (moves < 10) return "veri az";
  if (upShare >= 60) return "tek yönlü tırmanış";
  if (upShare <= 40) return "tek yönlü düşüş";
  return "dengeli salınım";
}

export function BucketPanel({
  city,
  product,
  tick,
  bucketHistory,
  onClose,
  onSelectCity,
}: Props) {
  const { data, error, loading } = useDetail<BucketDetail>(
    `/api/bucket/${city}/${product}`,
    tick,
  );

  const focus = data?.cities.find((c) => c.city === data.focus_city);

  return (
    <DetailShell
      eyebrow="ürün"
      title={data?.label ?? product}
      meta={data ? `${tickLabel(data.tick)} · katman ${data.tier}` : undefined}
      loading={loading}
      error={error}
      empty={!loading && !error && data == null}
      onClose={onClose}
    >
      {data && (
        <>
          <div className="dt__stats">
            <Stat
              label="pazar ortalaması"
              value={lira2(data.market_now_lira)}
              sub={`sezon başı ${lira2(data.market_initial_lira)}`}
            />
            <Stat
              label="sezon kayması"
              value={pct(data.market_drift_pct)}
              tone={data.market_drift_pct >= 0 ? "gain" : "loss"}
              sub="beş şehir ortalaması"
            />
            {focus && (
              <Stat
                label={`${focus.label} pazara göre`}
                value={pct(focus.vs_market_pct)}
                tone={focus.vs_market_pct >= 0 ? "loss" : "gain"}
                sub={focus.vs_market_pct >= 0 ? "burada pahalı" : "burada ucuz"}
              />
            )}
            <Stat
              label="hane basamağı"
              value={data.need_tier ?? "—"}
              sub={data.need_tier ? "hane bu malı alıyor" : "hane almıyor"}
            />
          </div>

          <Block
            title="zincirdeki yeri"
            note="girdisi ne, neye girdi oluyor — kıtlığın nereden geldiğini bu söyler"
          >
            <p className="dt__line">
              {data.is_raw ? (
                <>Ham madde — tarlada üretilir.</>
              ) : (
                <>
                  Girdileri:{" "}
                  {data.inputs.map(([name, p], i) => (
                    <span key={name}>
                      {i > 0 && " + "}
                      <strong>{name}</strong> (%{p})
                    </span>
                  ))}
                </>
              )}
            </p>
            <p className="dt__line">
              {data.feeds_into ? (
                <>
                  Şunun girdisi: <strong>{data.feeds_into}</strong>
                </>
              ) : (
                <>Zincirin sonu — doğrudan tüketiliyor.</>
              )}
            </p>
          </Block>

          <Block
            title="şehirlere göre"
            note="«pazara göre» sütunu arbitraj fırsatını gösterir: eksi olan şehirden alıp artı olana taşımak kâr"
            wide
          >
            <table className="dt__table">
              <thead>
                <tr>
                  <th>şehir</th>
                  <th className="num">sezon başı</th>
                  <th className="num">şimdi</th>
                  <th className="num">kayma</th>
                  <th className="num">dip / tepe</th>
                  <th>seyir</th>
                  <th className="num">pazara göre</th>
                  <th className="num">alış / satış</th>
                  <th className="num">üretici</th>
                  <th>grafik</th>
                </tr>
              </thead>
              <tbody>
                {data.cities.map((c) => {
                  const moves = c.up_ticks + c.down_ticks;
                  const isFocus = c.city === data.focus_city;
                  return (
                    <tr
                      key={c.city}
                      className={isFocus ? "dt__row--active" : undefined}
                      onClick={() => onSelectCity(c.city)}
                    >
                      <td>{c.label}</td>
                      <td className="num">{lira2(c.initial_lira)}</td>
                      <td className="num">{lira2(c.now_lira)}</td>
                      <td className={`num ${c.drift_pct >= 0 ? "dt__up" : "dt__down"}`}>
                        {pct(c.drift_pct)}
                      </td>
                      <td className="num">
                        {lira2(c.low_lira)} / {lira2(c.high_lira)}
                      </td>
                      <td>
                        {swingLabel(c.up_share_pct, moves)}
                        <span className="dt__dim">
                          {" "}
                          ↑{c.up_ticks} ↓{c.down_ticks}
                        </span>
                      </td>
                      <td className={`num ${c.vs_market_pct >= 0 ? "dt__down" : "dt__up"}`}>
                        {pct(c.vs_market_pct)}
                      </td>
                      <td className="num">
                        {c.bid_qty} / {c.ask_qty}
                      </td>
                      <td className="num">{c.producers}</td>
                      <td>
                        <Sparkline
                          values={bucketHistory[`${c.city}/${data.product}`] ?? []}
                          baseline={c.initial_lira}
                        />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </Block>
        </>
      )}
    </DetailShell>
  );
}
