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
 * Hücre seçimi burada yerel: ızgara zaten tek başına bir sayfa, seçili
 * hücre yalnız vurgulama için — dashboard'un odak durumunu kirletmiyor.
 */
interface Props {
  snapshot: Snapshot | null;
  bucketHistory: BucketHistory;
  onClose: () => void;
}

export function MarketGridPanel({ snapshot, bucketHistory, onClose }: Props) {
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
        onSelect={(city, product) => setSelected({ city, product })}
      />
      <p className="dt__note">
        Yeşil hücre baz fiyatın üstünde, kırmızı altında. Aynı ürünün
        şehirler arası farkı Tüccar'ın kâr alanı: fark ne kadar açıksa
        kervan o kadar değerli.
      </p>
    </DetailShell>
  );
}
