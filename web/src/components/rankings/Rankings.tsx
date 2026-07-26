import { useMemo, useState } from "react";
import type { PlayerDto, Snapshot } from "../../types";
import { compact, signedCompact } from "../../lib/format";
import { roleCode, roleColor, roleInk } from "../../lib/roles";
import "./rankings.css";

/**
 * Sol sütun — önce ekonominin dağılımı, sonra tek tek firmalar.
 *
 * Eskiden burada yalnız Sanayici satırları vardı: on satır, hepsi aynı
 * rozet. İzleyici "kim kazanıyor" sorusunu soramıyordu çünkü kazanan roller
 * tabloda hiç yoktu. Artık üstte rollerin kişi başı sonucu duruyor —
 * ekonominin kimin lehine işlediği tek bakışta okunuyor — altta ise
 * istenen role göre süzülebilen firma listesi.
 */

/** Rol filtresi düğmeleri. `null` = hepsi. */
const FILTERS: { key: string | null; label: string }[] = [
  { key: null, label: "hepsi" },
  { key: "Sanayici", label: "şirketler" },
  { key: "Tüccar", label: "tüccar" },
  { key: "Çiftçi", label: "çiftçi" },
];

interface Props {
  snapshot: Snapshot | null;
  prev: Snapshot | null;
  selectedId: number | null;
  onSelect: (id: number) => void;
}

export function Rankings({ snapshot, prev, selectedId, onSelect }: Props) {
  const [filter, setFilter] = useState<string | null>("Sanayici");

  const roles = snapshot?.roles ?? [];
  // Iraksak çubuk için ortak ölçek: en büyük mutlak değer iki yana da yeter.
  const roleScale = Math.max(1, ...roles.map((r) => Math.abs(r.per_capita_pnl_lira)));

  const rows = useMemo(() => {
    const all = snapshot?.leaderboard ?? [];
    return filter == null ? all : all.filter((p) => p.npc_kind === filter);
  }, [snapshot, filter]);

  const prevRank = useMemo(() => {
    const m = new Map<number, number>();
    const all = prev?.leaderboard ?? [];
    const list = filter == null ? all : all.filter((p) => p.npc_kind === filter);
    list.forEach((p, i) => m.set(p.id, i));
    return m;
  }, [prev, filter]);

  return (
    <section className="rank panel" aria-labelledby="rank-title">
      {/* ── Rol dağılımı ─────────────────────────────────────────────── */}
      <div className="panel__head">
        <h2 id="rank-title" className="panel__title">
          ROLLER
        </h2>
        <span className="panel__sub">kişi başı kâr</span>
      </div>

      <ul className="rank__roles">
        {roles.map((r) => {
          const pct = (Math.abs(r.per_capita_pnl_lira) / roleScale) * 50;
          const neg = r.per_capita_pnl_lira < 0;
          return (
            <li key={r.kind} className="rank__role">
              {/* Kimlik renkli işarette, okunabilirlik yazıda. Rol rengini
                  doğrudan yazıya vermek küçük puntoda 3.2:1'e düşüyordu. */}
              <span className="rank__role-name">
                <i
                  className="rank__swatch"
                  style={{ background: roleColor(r.label) }}
                  aria-hidden="true"
                />
                {r.label}
              </span>
              <span className="rank__role-n">×{r.count}</span>
              {/* Sıfır çizgisi ortada: sola zarar, sağa kâr. Yön tek bakışta. */}
              <span className="rank__bar" aria-hidden="true">
                <span className="rank__bar-zero" />
                <span
                  className={`rank__bar-fill${neg ? " rank__bar-fill--neg" : ""}`}
                  style={neg ? { right: "50%", width: `${pct}%` } : { left: "50%", width: `${pct}%` }}
                />
              </span>
              <span className={`rank__role-val${neg ? " is-neg" : ""}`}>
                {signedCompact(r.per_capita_pnl_lira)}
              </span>
            </li>
          );
        })}
        {roles.length === 0 && <li className="rank__empty">veri bekleniyor…</li>}
      </ul>

      {/* ── Firma sıralaması ─────────────────────────────────────────── */}
      <div className="panel__head rank__head2">
        <h2 className="panel__title">SIRALAMA</h2>
        <div className="rank__filters" role="group" aria-label="Rol filtresi">
          {FILTERS.map((f) => (
            <button
              key={f.label}
              type="button"
              className={`rank__filter${filter === f.key ? " is-on" : ""}`}
              aria-pressed={filter === f.key}
              onClick={() => setFilter(f.key)}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      <div className="rank__cols" aria-hidden="true">
        <span>#</span>
        <span>rol</span>
        <span>firma</span>
        <span className="rank__r">nakit</span>
        <span className="rank__r">PnL</span>
      </div>

      <ol className="rank__list">
        {rows.map((p, i) => (
          <RankRow
            key={p.id}
            rank={i + 1}
            delta={prevRank.has(p.id) ? (prevRank.get(p.id) as number) - i : 0}
            player={p}
            selected={selectedId === p.id}
            onSelect={() => onSelect(p.id)}
          />
        ))}
        {rows.length === 0 && <li className="rank__empty">bu rolde firma yok</li>}
      </ol>
    </section>
  );
}

function RankRow({
  rank,
  delta,
  player,
  selected,
  onSelect,
}: {
  rank: number;
  delta: number;
  player: PlayerDto;
  selected: boolean;
  onSelect: () => void;
}) {
  const neg = player.pnl_lira < 0;
  return (
    <li>
      <button
        type="button"
        className={`rank__row${selected ? " is-sel" : ""}`}
        onClick={onSelect}
        aria-current={selected ? "true" : undefined}
      >
        <span className="rank__pos">
          {rank}
          {delta !== 0 && (
            <span className={`rank__delta${delta > 0 ? " is-up" : " is-down"}`}>
              {delta > 0 ? "▲" : "▼"}
            </span>
          )}
        </span>
        {/* Rozet renk + kod taşıyor: kimlik yalnız renge bağlı kalmasın. */}
        <span className="rank__badge" style={{ color: roleInk(player.npc_kind) }}>
          {roleCode(player.npc_kind)}
        </span>
        <span className="rank__name">{player.name}</span>
        <span className="rank__cash">{compact(player.cash_lira)}</span>
        <span className={`rank__pnl${neg ? " is-neg" : ""}`}>
          {signedCompact(player.pnl_lira)}
        </span>
      </button>
    </li>
  );
}
