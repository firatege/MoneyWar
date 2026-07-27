#!/usr/bin/env bash
# Spekülatör ask ladder süpürmesi.
#
# Teşhis (bkz. memory/moneywar-spekulator-spread): Spek'in devir hızı %89 ama
# marjı sıfır. Alıcı taraf %2 işlem vergisi ödediği için bid %99 → efektif
# maliyet %100,98. Mevcut ladder envanter biriktikçe 97'ye indiği için zararına
# satıyor. Bu script ladder'ı efektif maliyetin üstüne çeken varyantları ölçer.
#
# Kullanım: ./scripts/spek_ladder_sweep.sh [oyun_sayisi] [tick]
set -euo pipefail

cd "$(dirname "$0")/.."

GAMES=${1:-40}
TICKS=${2:-350}
SRC=crates/moneywar-npc/src/behavior/roles/spekulator.rs
OUT=artifacts/spek-ladder
mkdir -p "$OUT"

# Değişiklikleri geri almak için orijinali sakla.
BACKUP=$(mktemp)
cp "$SRC" "$BACKUP"
restore() { cp "$BACKUP" "$SRC"; rm -f "$BACKUP"; }
trap restore EXIT

# "yuksek_stok orta_stok dusuk_stok" — stock>=100 / stock>=50 / else
VARIANTS=(
  "97 99 101"    # mevcut  — efektif maliyet %100,98'in altında, zararına satış
  "99 101 103"   # ihtiyatlı — taban maliyetin hemen üstü
  "101 103 105"  # agresif  — her basamakta pozitif marj
)

for v in "${VARIANTS[@]}"; do
  read -r hi mid lo <<<"$v"
  tag="${hi}_${mid}_${lo}"
  echo "=== ladder $hi/$mid/$lo ==="

  cp "$BACKUP" "$SRC"
  # ask_pct satırını tek seferde değiştir.
  sed -i "s|let ask_pct = if stock >= 100 { [0-9]* } else if stock >= 50 { [0-9]* } else { [0-9]* };|let ask_pct = if stock >= 100 { $hi } else if stock >= 50 { $mid } else { $lo };|" "$SRC"
  grep -q "{ $hi } else if stock >= 50 { $mid }" "$SRC" || { echo "sed tutmadı, satır değişmiş olabilir"; exit 1; }

  cargo build --release -p moneywar-sim 2>&1 | tail -2
  # --parallel yok: 40 paralel oyun sandbox'ı OOM ile düşürüyor.
  ./target/release/sim --games "$GAMES" --ticks "$TICKS" > "$OUT/$tag.txt" 2>&1

  echo "--- $tag özet ---"
  # ROL ADALETİ tablosu + makas satırı: kişi başı PnL, fill/emir, iflas hepsi burada.
  sed -n '/ROL ADALETİ/,/adalet makası/p' "$OUT/$tag.txt"
  grep -E "^  üretim:|birim eşleşti" "$OUT/$tag.txt"
  echo
done

echo "Tam çıktılar: $OUT/"
