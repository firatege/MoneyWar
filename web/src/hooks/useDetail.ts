import { useEffect, useRef, useState } from "react";

/**
 * Detay endpoint'ini çeker ve oyun ilerledikçe tazeler.
 *
 * Snapshot her tick WS'ten geliyor ama bu veriler orada yok — tıklanınca
 * HTTP'den çekiliyor. Açık kalan bir detay sayfası donmasın diye tick
 * ilerledikçe yeniden çekilir; ilk yüklemeden sonra gelen tazelemeler
 * eski veriyi ekranda tutar (yanıp sönme olmaz).
 *
 * `path` null ise istek atılmaz ve durum boşaltılır.
 */
export function useDetail<T>(path: string | null, tick: number, everyTicks = 3) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // Aynı yol için tekrar tekrar "yükleniyor" göstermemek adına ilk
  // yüklemeyi ayrı izliyoruz.
  const loadedPath = useRef<string | null>(null);

  const bucket = Math.floor(tick / Math.max(1, everyTicks));

  useEffect(() => {
    if (path == null) {
      setData(null);
      setError(null);
      loadedPath.current = null;
      return;
    }

    const isFirstLoad = loadedPath.current !== path;
    if (isFirstLoad) {
      setData(null);
      setLoading(true);
    }

    const ac = new AbortController();
    fetch(path, { signal: ac.signal })
      .then((r) => {
        if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
        return r.json() as Promise<T>;
      })
      .then((json) => {
        loadedPath.current = path;
        setData(json);
        setError(null);
      })
      .catch((e: unknown) => {
        if (ac.signal.aborted) return;
        // Tazeleme hatası açık sayfayı boşaltmasın — sadece ilk yükleme
        // başarısızsa kullanıcıya hata göster.
        if (loadedPath.current !== path) {
          setError(e instanceof Error ? e.message : "veri alınamadı");
        }
      })
      .finally(() => {
        if (!ac.signal.aborted) setLoading(false);
      });

    return () => ac.abort();
  }, [path, bucket]);

  return { data, error, loading };
}
