#!/usr/bin/env bash
# MoneyWar log analiz scripti
# Kullanım: ./scripts/analyze_log.sh [log_dosyasi]
# Argümansız çalışırsa debug/ altındaki en son log'u kullanır.

set -euo pipefail

LOG="${1:-}"
if [[ -z "$LOG" ]]; then
    LOG=$(ls -t "$(dirname "$0")/../debug/"*.log 2>/dev/null | head -1)
    if [[ -z "$LOG" ]]; then
        echo "Hata: log dosyası bulunamadı. debug/ klasörünü kontrol et." >&2
        exit 1
    fi
fi

if [[ ! -f "$LOG" ]]; then
    echo "Hata: $LOG bulunamadı." >&2
    exit 1
fi

EPOCH=$(basename "$LOG" .log | sed 's/moneywar-//')

echo "════════════════════════════════════════════════════════════"
echo " MoneyWar Analiz — $LOG"
echo "════════════════════════════════════════════════════════════"
echo ""

# ── HEADER ──────────────────────────────────────────────────────
echo "▶ OYUN AYARLARI"
head -8 "$LOG" | grep "^#" | sed 's/^# /  /'
echo ""

# Son tick
LAST_TICK=$(grep -oP '^t\s+\K\d+' "$LOG" | sort -n | tail -1)
echo "  Son tick: $LAST_TICK"
echo ""

# ── OYUNCU ROLLERİ ──────────────────────────────────────────────
echo "▶ OYUNCU ROLLERİ"
echo "  Tüccar  (PLY-100..103): Halil/Cemal/Yusuf/Fatma"
echo "  Sanayici (PLY-104..106): Şerife/Ahmet/Ayşe"
echo "  Alıcı   (PLY-107..116): Maaşlı tüketici"
echo "  Çiftçi  (PLY-117+)    : Hammadde üretici"
echo ""

# ── ÜRETİM ──────────────────────────────────────────────────────
echo "▶ ÜRETİM (ProductionStarted)"
grep "ProductionStarted" "$LOG" | grep -oP "^t\s+\d+\s+\K\w[\w ]+" | \
    sed 's/[[:space:]]*$//' | sort | uniq -c | sort -rn | \
    while read count name; do printf "  %-22s %4d çalışma\n" "$name" "$count"; done

echo ""
echo "  Ürün bazlı:"
grep "ProductionStarted" "$LOG" | grep -oP "product: \w+" | sort | uniq -c | sort -rn | \
    while read count prod; do printf "    %-14s %4d\n" "$prod:" "$count"; done

echo ""
echo "  FactoryIdle (hammadde yetersizliği):"
grep "FactoryIdle" "$LOG" | grep -oP "^t\s+\d+\s+\K\w[\w ]+" | \
    sed 's/[[:space:]]*$//' | sort | uniq -c | sort -rn | \
    while read count name; do printf "    %-22s %4d idle\n" "$name" "$count"; done

echo ""
echo "  En sık FactoryIdle nedenleri (ilk 10):"
grep "FactoryIdle" "$LOG" | grep -oP 'reason: "[^"]+"' | sort | uniq -c | sort -rn | head -10 | \
    while read count reason; do printf "    %4d × %s\n" "$count" "$reason"; done
echo ""

# ── FABRIKA MALİYETLERİ ─────────────────────────────────────────
echo "▶ FABRIKA YATIRIMLARI"
grep "FactoryBuilt" "$LOG" | grep -oP "owner: PlayerId\(\d+\), city: \w+, product: \w+, cost: Money\(\d+\)" | \
    while IFS= read -r line; do
        pid=$(echo "$line" | grep -oP "PlayerId\(\K\d+")
        city=$(echo "$line" | grep -oP "city: \K\w+")
        prod=$(echo "$line" | grep -oP "product: \K\w+")
        cost=$(echo "$line" | grep -oP "cost: Money\(\K\d+")
        lira=$((cost / 100))
        printf "  PLY-%-3s %10s/%-12s %6d₺\n" "$pid" "$city" "$prod" "$lira"
    done
echo ""

# ── EŞLEŞİM ORANLARI ────────────────────────────────────────────
echo "▶ EŞLEŞİM ORANLARI (MarketCleared)"

# Genel
total_buy=$(grep "MarketCleared" "$LOG" | grep -oP "submitted_buy_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s}')
total_sell=$(grep "MarketCleared" "$LOG" | grep -oP "submitted_sell_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s}')
total_matched=$(grep "MarketCleared" "$LOG" | grep -oP "matched_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s}')
echo "  Genel: $total_matched eşleşti / $total_buy BUY gönderildi = $(echo "scale=1; $total_matched * 100 / $total_buy" | bc)%"
echo ""

printf "  %-14s %8s %8s %8s %8s %8s\n" "Ürün" "Eşleşen" "BUY-sub" "SELL-sub" "BUY-%" "SELL-%"
for prod in Pamuk Bugday Zeytin Kumas Un Zeytinyagi; do
    m=$(grep "MarketCleared.*product: $prod" "$LOG" | grep -oP "matched_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    b=$(grep "MarketCleared.*product: $prod" "$LOG" | grep -oP "submitted_buy_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    sv=$(grep "MarketCleared.*product: $prod" "$LOG" | grep -oP "submitted_sell_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    buy_pct=$([ "$b" -gt 0 ] && echo "scale=1; $m * 100 / $b" | bc || echo "0")
    sell_pct=$([ "$sv" -gt 0 ] && echo "scale=1; $m * 100 / $sv" | bc || echo "0")
    printf "  %-14s %8d %8d %8d %7s%% %7s%%\n" "$prod" "$m" "$b" "$sv" "$buy_pct" "$sell_pct"
done
echo ""

# ── FİYAT ANALİZİ ───────────────────────────────────────────────
echo "▶ FİYAT ARALIĞI (tüm oyun fill'leri)"
printf "  %-14s %8s %8s %8s %8s\n" "Ürün" "Min" "Max" "Ort" "Adet"
for prod in Pamuk Bugday Zeytin Kumas Un Zeytinyagi; do
    stats=$(grep "OrderMatched.*product: $prod" "$LOG" | grep -oP "price: Money\(\d+\)" | grep -oP "\d+" | \
        awk '{if(NR==1)min=$1; if($1<min)min=$1; if($1>max)max=$1; sum+=$1; count++} END{if(count>0) printf "%.2f %.2f %.2f %d\n", min/100, max/100, sum/count/100, count}')
    if [[ -n "$stats" ]]; then
        read mn mx av cnt <<< "$stats"
        printf "  %-14s %7s₺ %7s₺ %7s₺ %8s\n" "$prod" "$mn" "$mx" "$av" "$cnt"
    fi
done
echo ""

# ── SON TİCK FİYAT/STOK DURUMU ──────────────────────────────────
echo "▶ SON TİCK PİYASA DURUMU (t$LAST_TICK bucket)"
grep "^t $LAST_TICK.*<bucket>" "$LOG" | \
    sed "s/^t $LAST_TICK  <bucket>                 /  /" | \
    sed 's/  */ /g'
echo ""

# ── TÜCCARın AL-SAT NET'İ ──────────────────────────────────────
echo "▶ TÜCCAR AL-SAT ANALİZİ (PLY-100..103)"
printf "  %-8s %10s %10s %10s %8s %8s\n" "Oyuncu" "Satış(₺)" "Alış(₺)" "Net(₺)" "Aldı(u)" "Sattı(u)"
for pid in 100 101 102 103; do
    rev=$(grep "OrderMatched" "$LOG" | grep "seller: PlayerId($pid)" | \
        grep -oP "quantity: \d+, price: Money\(\d+\)" | \
        sed 's/quantity: \([0-9]*\), price: Money(\([0-9]*\))/\1 \2/' | \
        awk '{sum += $1 * $2} END {printf "%.0f\n", sum/100}')
    cost=$(grep "OrderMatched" "$LOG" | grep "buyer: PlayerId($pid)" | \
        grep -oP "quantity: \d+, price: Money\(\d+\)" | \
        sed 's/quantity: \([0-9]*\), price: Money(\([0-9]*\))/\1 \2/' | \
        awk '{sum += $1 * $2} END {printf "%.0f\n", sum/100}')
    buy_u=$(grep "OrderMatched" "$LOG" | grep "buyer: PlayerId($pid)" | \
        grep -oP "quantity: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    sell_u=$(grep "OrderMatched" "$LOG" | grep "seller: PlayerId($pid)" | \
        grep -oP "quantity: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    net=$((${rev:-0} - ${cost:-0}))
    printf "  PLY-%-4s %10s %10s %10s %8s %8s\n" "$pid" "$rev" "$cost" "$net" "$buy_u" "$sell_u"
done

echo ""
echo "  Tüccar ürün bazlı satış (SELL fills):"
for pid in 100 101 102 103; do
    echo "    PLY-$pid:"
    grep "OrderMatched" "$LOG" | grep "seller: PlayerId($pid)" | \
        grep -oP "product: \w+" | sort | uniq -c | \
        while read c p; do printf "      %-14s %4d fill\n" "$p" "$c"; done
done
echo ""

# ── SANAYİCİ AL-SAT NET'İ ───────────────────────────────────────
echo "▶ SANAYİCİ AL-SAT ANALİZİ (PLY-104..106)"
printf "  %-8s %10s %10s %10s %8s %8s\n" "Oyuncu" "Satış(₺)" "Alış(₺)" "Net(₺)" "Aldı(u)" "Sattı(u)"
for pid in 104 105 106; do
    rev=$(grep "OrderMatched" "$LOG" | grep "seller: PlayerId($pid)" | \
        grep -oP "quantity: \d+, price: Money\(\d+\)" | \
        sed 's/quantity: \([0-9]*\), price: Money(\([0-9]*\))/\1 \2/' | \
        awk '{sum += $1 * $2} END {printf "%.0f\n", sum/100}')
    cost=$(grep "OrderMatched" "$LOG" | grep "buyer: PlayerId($pid)" | \
        grep -oP "quantity: \d+, price: Money\(\d+\)" | \
        sed 's/quantity: \([0-9]*\), price: Money(\([0-9]*\))/\1 \2/' | \
        awk '{sum += $1 * $2} END {printf "%.0f\n", sum/100}')
    buy_u=$(grep "OrderMatched" "$LOG" | grep "buyer: PlayerId($pid)" | \
        grep -oP "quantity: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    sell_u=$(grep "OrderMatched" "$LOG" | grep "seller: PlayerId($pid)" | \
        grep -oP "quantity: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    # Factory cost toplam
    fab_cost=$(grep "FactoryBuilt" "$LOG" | grep "owner: PlayerId($pid)" | \
        grep -oP "cost: Money\(\d+\)" | grep -oP "\d+" | awk '{s+=$1}END{printf "%.0f\n", s/100}')
    net=$((${rev:-0} - ${cost:-0} - ${fab_cost:-0}))
    printf "  PLY-%-4s %10s %10s fab:%-6s net:%-8s sold:%s\n" "$pid" "$rev" "$cost" "$fab_cost" "$net" "$sell_u"
done

echo ""
echo "  Sanayici mamul satış dökümü:"
for pid in 104 105 106; do
    echo "    PLY-$pid SELL:"
    grep "OrderMatched" "$LOG" | grep "seller: PlayerId($pid)" | \
        grep -oP "product: \w+" | sort | uniq -c | \
        while read c p; do printf "      %-14s %4d fill\n" "$p" "$c"; done
    echo "    PLY-$pid BUY:"
    grep "OrderMatched" "$LOG" | grep "buyer: PlayerId($pid)" | \
        grep -oP "product: \w+" | sort | uniq -c | \
        while read c p; do printf "      %-14s %4d fill\n" "$p" "$c"; done
done
echo ""

# ── ŞEHIR STOK ÖZETI (son tick) ─────────────────────────────────
echo "▶ ŞEHİR STOK DURUMU (t$LAST_TICK)"
grep "^t $LAST_TICK.*<snap-" "$LOG" | \
    sed "s/^t $LAST_TICK  /  /" | \
    sed 's/ | last.*//'
echo ""

# ── ÖZET ────────────────────────────────────────────────────────
echo "════════════════════════════════════════════════════════════"
echo " ÖZET"
echo "════════════════════════════════════════════════════════════"

total_prod=$(grep "ProductionStarted" "$LOG" | wc -l)
total_idle=$(grep "FactoryIdle" "$LOG" | wc -l)
total_fill=$(grep "OrderMatched" "$LOG" | wc -l)
total_exp=$(grep "OrderExpired" "$LOG" | wc -l)

echo "  Toplam üretim çalışması : $total_prod"
echo "  Toplam fabrika idle     : $total_idle"
echo "  Toplam eşleşme          : $total_fill fill"
echo "  Toplam expired          : $total_exp emir"
echo ""

# Kümülatif match rate
echo "  Mamul BUY karşılanma oranı:"
for prod in Kumas Un Zeytinyagi; do
    m=$(grep "MarketCleared.*product: $prod" "$LOG" | grep -oP "matched_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    b=$(grep "MarketCleared.*product: $prod" "$LOG" | grep -oP "submitted_buy_qty: \d+" | grep -oP "\d+" | awk '{s+=$1}END{print s+0}')
    pct=$([ "$b" -gt 0 ] && echo "scale=1; $m * 100 / $b" | bc || echo "0")
    printf "    %-12s %s%%\n" "$prod:" "$pct"
done
echo ""
