import { useState } from "react";
import type { Snapshot } from "../types";
import type { BucketHistory } from "../hooks/useGameSocket";
import { PriceGrid } from "../components/price-grid/PriceGrid";
import { DetailShell } from "./DetailShell";

/**
 * Piyasa geneli — şehir × ürün fiyat ızgarası.
 *
 * Izgara harita gelmeden önce ana ekrandaydı ve harita için kaldırılmıştı.
 * İkisi farklı soruları cevaplıyor, biri diğerinin yerine geçmiyor:
 * harita **nerede ne oluyor** (coğrafya, fabrika yoğunluğu, olaylar),
 * ızgara **fiyatlar nerede ayrışıyor** (55 hücrenin tamamı tek bakışta,
 * arbitraj boşlukları dahil). O yüzden ızgara silinmek yerine kendi
 * sayfasına taşındı.
 *
 * Hücre seçimi burada yerel (vurgulama için), ama **tıklama ürün sayfasını
 * açar**: hücre tek başına yalnız anlık fiyatı gösteriyordu ve "26₺ pahalı mı
 * ucuz mu" sorusunu cevaplamıyordu. Cevap üç referans gerektiriyor — sezon
 * başı, pazar ortalaması, kendi seyri — ve üçü de ürün sayfasında.
 */
interface Props {
  snapshot: Snapshot | null;
  bucketHistory: BucketHistory;
  onClose: () => void;
  onOpenBucket: (city: string, product: string) => void;
}

export function MarketGridPanel({ snapshot, bucketHistory, onClose, onOpenBucket }: Props) {
  const [selected, setSelected] = useState({ city: "istanbul", product: "pamuk" });

  return (
    <DetailShell
      eyebrow="piyasa geneli"
      title="Fiyat ızgarası"
      meta={<span className="dt__note">her şehir × her ürün</span>}
      loading={false}
      error={null}
      onClose={onClose}
    >
      <PriceGrid
        snapshot={snapshot}
        bucketHistory={bucketHistory}
        selected={selected}
        onSelect={(city, product) => {
          setSelected({ city, product });
          onOpenBucket(city, product);
        }}
      />
      <p className="dt__note">
        Yeşil hücre baz fiyatın üstünde, kırmızı altında. Aynı ürünün
        şehirler arası farkı Tüccar'ın kâr alanı: fark ne kadar açıksa
        kervan o kadar değerli.
      </p>
    </DetailShell>
  );
}
