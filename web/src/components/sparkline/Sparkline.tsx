interface Props {
  values: number[];
  width?: number;
  height?: number;
  /** Yön rengini belirleyen referans (yoksa ilk değer). */
  baseline?: number;
}

/**
 * Hafif SVG sparkline — ızgara hücreleri için. lightweight-charts yerine
 * basit polyline (30 hücre × her tick performans). Son değer referansın
 * üstündeyse yeşil, altındaysa kırmızı; alan hafif doldurulur.
 */
export function Sparkline({ values, width = 100, height = 26, baseline }: Props) {
  if (values.length < 2) {
    return <svg className="spark" viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" />;
  }

  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const pad = 2;
  const innerH = height - pad * 2;
  const step = width / (values.length - 1);

  const xy = (v: number, i: number): [number, number] => {
    const x = i * step;
    const y = pad + innerH - ((v - min) / range) * innerH;
    return [x, y];
  };

  const ref = baseline ?? values[0];
  const up = values[values.length - 1] >= ref;
  const color = up ? "var(--gain)" : "var(--loss)";

  const linePts = values.map((v, i) => xy(v, i).join(",")).join(" ");
  const [, y0] = xy(values[0], 0);
  const areaPts = `0,${height} ${linePts} ${width},${height}`;
  void y0;

  return (
    <svg
      className="spark"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <polygon points={areaPts} fill={color} fillOpacity="0.1" stroke="none" />
      <polyline
        points={linePts}
        fill="none"
        stroke={color}
        strokeWidth="1.25"
        strokeLinejoin="round"
        strokeLinecap="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
