import { useMemo } from "react";
import type { Snapshot } from "../../types";
import type { MarketPoint } from "../../lib/derive";
import { compact } from "../../lib/format";
import { PRODUCTS } from "../../lib/catalog";
import { Sparkline } from "../sparkline/Sparkline";
import "./chart-strip.css";

/**
 * Alt şerit — ekonominin dört sorusu, dört ayrı biçimde.
 *
 * Her kart bilinçli olarak farklı bir biçim kullanıyor; hepsi çubuk olsaydı
 * göz hangisinin neyi ölçtüğünü ayırt edemezdi:
 *
 *   · para arzı → tek büyük sayı + trend (tek değerin işi bu)
 *   · fabrikalar → waffle, her kare bir fabrika (sayılabilir şey sayılsın)
 *   · istihdam → ölçer, tavana karşı tek oran
 *   · üretim zinciri → katman katman, çünkü soru "zincir nerede tıkanıyor"
 */

interface Props {
  snapshot: Snapshot | null;
  market: MarketPoint[];
}

export function ChartStrip({ snapshot, market }: Props) {
  const eco = snapshot?.economy;

  const indexSeries = useMemo(() => market.slice(-60).map((m) => m.index), [market]);
  const volumeSeries = useMemo(() => market.slice(-60).map((m) => m.volume), [market]);

  /** Üretim zinciri: katman başına fabrika ve kaçı atıl. */
  const tiers = useMemo(() => {
    const tierOf = new Map(PRODUCTS.map((p) => [p.slug, p.tier]));
    const acc = new Map<number, { total: number; idle: number }>([
      [1, { total: 0, idle: 0 }],
      [2, { total: 0, idle: 0 }],
      [3, { total: 0, idle: 0 }],
    ]);
    for (const f of snapshot?.factories ?? []) {
      const t = tierOf.get(f.product);
      if (t == null || t === 0) continue;
      const e = acc.get(t);
      if (!e) continue;
      e.total += 1;
      if (f.idle) e.idle += 1;
    }
    return [1, 2, 3].map((t) => ({ tier: t, ...(acc.get(t) as { total: number; idle: number }) }));
  }, [snapshot]);

  const factories = snapshot?.factories ?? [];
  const idleCount = eco?.factories_idle ?? 0;
  const activeCount = eco?.factories_active ?? 0;
  const employed = eco?.employed ?? 0;
  const pool = eco?.labor_pool ?? 0;
  const gini = eco?.wealth_gini ?? 0;

  return (
    <section className="strip" aria-label="Ekonomi göstergeleri">
      {/* ── Para arzı: tek sayı, tek trend ───────────────────────────── */}
      <article className="strip__card">
        <h3 className="strip__title">Para arzı</h3>
        <p className="strip__hero">
          {eco ? compact(eco.money_supply_lira) : "—"}
          <span className="strip__unit">₺</span>
        </p>
        <div className="strip__spark">
          <Sparkline values={indexSeries} width={140} height={30} baseline={100} />
        </div>
        <p className="strip__foot">
          fiyat endeksi ·{" "}
          <strong>{indexSeries.length ? indexSeries[indexSeries.length - 1].toFixed(0) : "—"}</strong>{" "}
          <span className="strip__muted">(baz 100)</span>
        </p>
      </article>

      {/* ── Fabrikalar: waffle — her kare bir fabrika ─────────────────── */}
      <article className="strip__card">
        <h3 className="strip__title">Fabrikalar</h3>
        <p className="strip__hero strip__hero--sm">
          {activeCount}
          <span className="strip__unit">/ {activeCount + idleCount} çalışıyor</span>
        </p>
        <ul className="strip__waffle" aria-hidden="true">
          {factories.map((f) => (
            <li
              key={f.id}
              className={`strip__cell${f.idle ? " is-idle" : ""}`}
              title={`${f.product} · ${f.idle ? "atıl" : "çalışıyor"}`}
            />
          ))}
        </ul>
        <p className="strip__foot">
          <span className="strip__key strip__key--on" /> çalışan
          <span className="strip__key strip__key--off" /> atıl
        </p>
      </article>

      {/* ── İstihdam: tavana karşı tek oran → ölçer ───────────────────── */}
      <article className="strip__card">
        <h3 className="strip__title">İstihdam</h3>
        <p className="strip__hero strip__hero--sm">
          {employed}
          <span className="strip__unit">/ {pool} işçi</span>
        </p>
        <div
          className="strip__meter"
          role="meter"
          aria-valuenow={employed}
          aria-valuemin={0}
          aria-valuemax={pool || 1}
          aria-label="Çalışan işçi / işgücü havuzu"
        >
          <span
            className="strip__meter-fill"
            style={{ width: `${pool > 0 ? (employed / pool) * 100 : 0}%` }}
          />
        </div>
        <p className="strip__foot">
          havuzun <strong>%{pool > 0 ? Math.round((employed / pool) * 100) : 0}</strong>&apos;i
          işte
        </p>
      </article>

      {/* ── Servet dağılımı: tek oran, anlamı etiketle ────────────────── */}
      <article className="strip__card">
        <h3 className="strip__title">Servet dağılımı</h3>
        <p className="strip__hero strip__hero--sm">
          {gini.toFixed(2)}
          <span className="strip__unit">Gini</span>
        </p>
        <div
          className="strip__meter"
          role="meter"
          aria-valuenow={Number(gini.toFixed(2))}
          aria-valuemin={0}
          aria-valuemax={1}
          aria-label="Servet Gini katsayısı"
        >
          <span className="strip__meter-fill strip__meter-fill--warn" style={{ width: `${gini * 100}%` }} />
        </div>
        <p className="strip__foot">
          {gini < 0.3 ? "dengeli" : gini < 0.5 ? "makas açılıyor" : "servet tek elde"}
          <span className="strip__muted"> · 0 eşit, 1 tek elde</span>
        </p>
      </article>

      {/* ── Üretim zinciri: katman katman ─────────────────────────────── */}
      <article className="strip__card strip__card--wide">
        <h3 className="strip__title">Üretim zinciri</h3>
        <ul className="strip__tiers">
          {tiers.map((t) => {
            const running = t.total - t.idle;
            return (
              <li key={t.tier} className="strip__tier">
                <span className="strip__tier-name">{t.tier}. katman</span>
                <span className="strip__tier-bar">
                  {t.total === 0 ? (
                    <span className="strip__tier-empty">fabrika yok</span>
                  ) : (
                    <>
                      {/* Parça-bütün: çalışan + atıl = toplam. Aralarında
                          yüzey boşluğu var ki iki parça birbirine yapışmasın. */}
                      <span
                        className="strip__tier-fill"
                        style={{ width: `${(running / t.total) * 100}%` }}
                      />
                      <span
                        className="strip__tier-fill strip__tier-fill--idle"
                        style={{ width: `${(t.idle / t.total) * 100}%` }}
                      />
                    </>
                  )}
                </span>
                <span className="strip__tier-num">
                  {running}/{t.total}
                </span>
              </li>
            );
          })}
        </ul>
        <p className="strip__foot">
          işlem hacmi{" "}
          <strong>{volumeSeries.length ? volumeSeries[volumeSeries.length - 1] : 0}</strong> birim/tick
        </p>
      </article>
    </section>
  );
}
