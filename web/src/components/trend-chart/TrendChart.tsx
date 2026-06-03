import { useEffect, useLayoutEffect, useRef } from "react";
import {
  createChart,
  type IChartApi,
  type ISeriesApi,
  type UTCTimestamp,
} from "lightweight-charts";
import "./trend-chart.css";

export interface TrendPoint {
  tick: number;
  value: number;
}

interface Props {
  points: TrendPoint[];
  /** Yeşil/kırmızı ayrım eşiği (fiyat baseline'ı veya PnL için 0). */
  baseline: number;
  emptyText?: string;
}

function toTime(tick: number): UTCTimestamp {
  return tick as UTCTimestamp;
}

/**
 * Stock tarzı artış/azalış grafiği — baseline üstü yeşil, altı kırmızı alan.
 * lightweight-charts BaselineSeries kullanır. Canvas container koşulsuz
 * mount edilir (aksi halde createChart ref'i null bulup grafik hiç oluşmaz).
 */
export function TrendChart({ points, baseline, emptyText = "veri bekleniyor…" }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRef = useRef<ISeriesApi<"Baseline"> | null>(null);

  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const chart = createChart(el, {
      layout: {
        background: { color: "transparent" },
        textColor: "rgba(150,150,160,0.65)",
        fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
        fontSize: 11,
      },
      grid: {
        vertLines: { color: "rgba(55,57,66,0.4)" },
        horzLines: { color: "rgba(55,57,66,0.4)" },
      },
      crosshair: {
        vertLine: { color: "rgba(130,130,150,0.5)", width: 1 },
        horzLine: { color: "rgba(130,130,150,0.5)", width: 1 },
      },
      rightPriceScale: { borderColor: "rgba(55,57,66,0.5)" },
      timeScale: {
        borderColor: "rgba(55,57,66,0.5)",
        // Tick numarasını göster — lightweight-charts tarih formatını bypass et.
        tickMarkFormatter: (t: UTCTimestamp) => `t${t}`,
        timeVisible: false,
        secondsVisible: false,
      },
      localization: {
        // Tarih yerine tick etiketini kullan.
        timeFormatter: (t: UTCTimestamp) => `tick ${t}`,
      },
      width: el.clientWidth,
      height: el.clientHeight,
      handleScroll: false,
      handleScale: false,
    });

    const series = chart.addBaselineSeries({
      baseValue: { type: "price", price: baseline },
      topLineColor: "rgba(110,190,140,0.95)",
      topFillColor1: "rgba(110,190,140,0.28)",
      topFillColor2: "rgba(110,190,140,0.02)",
      bottomLineColor: "rgba(200,110,95,0.95)",
      bottomFillColor1: "rgba(200,110,95,0.02)",
      bottomFillColor2: "rgba(200,110,95,0.28)",
      lineWidth: 2,
      priceLineVisible: false,
      lastValueVisible: true,
    });

    chartRef.current = chart;
    seriesRef.current = series;

    const ro = new ResizeObserver(() => {
      chart.resize(el.clientWidth, el.clientHeight);
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      chart.remove();
      chartRef.current = null;
      seriesRef.current = null;
    };
  }, []);

  // baseline değişince series base value güncelle.
  useEffect(() => {
    seriesRef.current?.applyOptions({
      baseValue: { type: "price", price: baseline },
    });
  }, [baseline]);

  useEffect(() => {
    const series = seriesRef.current;
    if (!series || points.length === 0) return;
    series.setData(points.map((p) => ({ time: toTime(p.tick), value: p.value })));
    chartRef.current?.timeScale().fitContent();
  }, [points]);

  return (
    <div className="trend">
      <div className="trend__canvas" ref={containerRef} />
      {points.length === 0 && <div className="trend__overlay">{emptyText}</div>}
    </div>
  );
}
