import { useEffect, useLayoutEffect, useRef } from "react";
import {
  createChart,
  type IChartApi,
  type ISeriesApi,
  type LineData,
  type UTCTimestamp,
} from "lightweight-charts";
import type { PriceCell, PricePoint } from "../../types";
import { lira2 } from "../../lib/format";
import "./market-chart.css";

interface Props {
  cell: PriceCell | null;
  points: PricePoint[];
}

// tick sayısını UTCTimestamp olarak iletiyoruz — timeScale.tickMarkFormatter ile t-prefix.
function toTime(tick: number): UTCTimestamp {
  return tick as UTCTimestamp;
}

export function MarketChart({ cell, points }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRef = useRef<ISeriesApi<"Line"> | null>(null);
  const baselineRef = useRef<ISeriesApi<"Line"> | null>(null);

  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const chart = createChart(el, {
      layout: {
        background: { color: "transparent" },
        textColor: "rgba(170,170,180,0.7)",
        fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
        fontSize: 11,
      },
      grid: {
        vertLines: { color: "rgba(60,62,72,0.5)" },
        horzLines: { color: "rgba(60,62,72,0.5)" },
      },
      crosshair: {
        vertLine: { color: "rgba(140,140,160,0.6)", width: 1 },
        horzLine: { color: "rgba(140,140,160,0.6)", width: 1 },
      },
      rightPriceScale: {
        borderColor: "rgba(60,62,72,0.6)",
        textColor: "rgba(140,140,160,0.7)",
      },
      timeScale: {
        borderColor: "rgba(60,62,72,0.6)",
        tickMarkFormatter: (t: UTCTimestamp) => `t${t}`,
      },
      width: el.clientWidth,
      height: el.clientHeight,
      handleScroll: false,
      handleScale: false,
    });

    const series = chart.addLineSeries({
      color: "rgba(160,140,90,0.9)",
      lineWidth: 2,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: true,
      crosshairMarkerRadius: 3,
    });

    const baseline = chart.addLineSeries({
      color: "rgba(80,82,90,0.7)",
      lineWidth: 1,
      lineStyle: 2, // Dashed
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
    });

    chartRef.current = chart;
    seriesRef.current = series;
    baselineRef.current = baseline;

    const ro = new ResizeObserver(() => {
      chart.resize(el.clientWidth, el.clientHeight);
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      chart.remove();
      chartRef.current = null;
      seriesRef.current = null;
      baselineRef.current = null;
    };
  }, []);

  useEffect(() => {
    const series = seriesRef.current;
    const baseline = baselineRef.current;
    if (!series || !baseline || points.length === 0) return;

    const lineData: LineData[] = points.map((p) => ({
      time: toTime(p.tick),
      value: p.lira,
    }));
    series.setData(lineData);

    if (cell?.baseline_lira != null) {
      const base = cell.baseline_lira;
      baseline.setData(
        points.map((p) => ({ time: toTime(p.tick), value: base })),
      );
    }

    chartRef.current?.timeScale().fitContent();
  }, [points, cell]);

  if (!cell) {
    return (
      <div className="mc mc--empty">
        <span>ürün seçin</span>
      </div>
    );
  }

  const pct =
    cell.last_lira != null && cell.baseline_lira > 0
      ? ((cell.last_lira - cell.baseline_lira) / cell.baseline_lira) * 100
      : null;
  const dir =
    pct == null ? "flat" : pct > 0.05 ? "up" : pct < -0.05 ? "down" : "flat";

  return (
    <div className="mc">
      <div className="mc__head">
        <div className="mc__id">
          <span className="mc__product">{cell.product_label}</span>
          <span className="mc__city">{cell.city_label}</span>
          <span className="mc__raw-badge">{cell.is_raw ? "HAM" : "MAMUL"}</span>
        </div>
        <div className="mc__stats">
          <span className="mc__stat">
            <span className="mc__stat-l">SON</span>
            <span className="mc__stat-v num">{lira2(cell.last_lira)}</span>
          </span>
          <span className="mc__stat">
            <span className="mc__stat-l">ORT5</span>
            <span className="mc__stat-v num">{lira2(cell.avg5_lira)}</span>
          </span>
          <span className="mc__stat">
            <span className="mc__stat-l">BAZE</span>
            <span className="mc__stat-v num">{lira2(cell.baseline_lira)}</span>
          </span>
          {pct != null && (
            <span className={`mc__delta mc__delta--${dir}`}>
              {pct > 0 ? "+" : ""}
              {pct.toFixed(1)}%
            </span>
          )}
        </div>
      </div>
      <div className="mc__canvas" ref={containerRef} />
      <div className="mc__foot">
        <span>
          AL: {cell.buy_qty} · SAT: {cell.sell_qty}
        </span>
        <span>
          BID: {lira2(cell.bid_lira)} · ASK: {lira2(cell.ask_lira)}
        </span>
      </div>
    </div>
  );
}
