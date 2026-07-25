import { useEffect } from "react";
import { Logo } from "../brand/Logo";
import "./help.css";

interface Props {
  onClose: () => void;
}

/** Üretim zinciri katmanları — girdiler → çıktı. */
const CHAIN: Array<[string, string]> = [
  ["Pamuk · Buğday · Zeytin · Boya · Üzüm", "hammadde (tarlada)"],
  ["Pamuk → Kumaş · Buğday → Un · Zeytin → Zeytinyağı · Üzüm → Şarap", "tek girdi"],
  ["Kumaş + Boya → Elbise · Un + Zeytinyağı → Ekmek", "iki girdi"],
  ["Ekmek + Şarap + Zeytinyağı → Ziyafet Sofrası", "üç girdi — en kârlısı"],
];

const PANELS: Array<[string, string]> = [
  ["SIRALAMA", "firmalar kâra (PnL) göre dizilir — bir firmaya tıkla, detayını ve PnL seyrini gör"],
  ["HARİTA", "şehirler, oradaki firmalar ve kervanlar — 👑 tekel, kırmızı nabız savaş, 🔒 tedariki kesilmiş fabrika; şehre tıkla, firmalarını gör"],
  ["FİYAT IZGARASI", "5 şehir × 12 ürün; her hücre o malın fiyat trendi (sparkline) — tıkla, emir defterini aç"],
  ["PİYASA GENELİ", "endeks (tüm fiyatların baz fiyata oranı ×100; 100 üstü = piyasa sıcak) + işlem hacmi"],
  ["AKIŞ", "entrika akışı: kim hangi pazarı ele geçirdi, kim kime savaş açtı, kim battı — düğmeyle sıradan işlemleri de açabilirsin"],
  ["HABER", "en çok yükselen/düşen mallar ve sıralamada yükselen firmalar"],
];

export function HelpOverlay({ onClose }: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="help" onClick={onClose}>
      <div className="help__panel" onClick={(e) => e.stopPropagation()}>
        <div className="help__head">
          <div className="help__brand">
            <Logo size={20} />
            <span className="help__brand-name">MoneyWar nedir?</span>
          </div>
          <button className="help__close" onClick={onClose} aria-label="kapat">
            kapat ✕
          </button>
        </div>

        <div className="help__body">
          <p className="help__lead">
            Sonsuza dek dönen bir <b>ekonomi simülasyonu</b>. Yapay firmalar 5 şehirde
            hammadde üretir, işler ve satar — ama asıl mesele rekabet: pazarları
            tekelleştirir, birbirinin fiyatını kırar, rakibinin tedarikini keser ve
            batırır. Sen izlersin: <b>kim ne yapıyor</b>, haritada ve akışta.
          </p>

          <section className="help__sec">
            <h3 className="help__sec-title">ÜRETİM ZİNCİRİ</h3>
            <div className="help__chain">
              {CHAIN.map(([raw, fin]) => (
                <div className="help__chain-row" key={raw}>
                  <span className="help__raw">{raw}</span>
                  <span className="help__arrow">·</span>
                  <span className="help__fin">{fin}</span>
                </div>
              ))}
            </div>
            <p className="help__note">
              Çiftçi hammadde üretir · Sanayici fabrikada mamule çevirir · Tüccar
              şehirler arası taşıyıp arbitraj yapar.
            </p>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">ŞEHİRLER</h3>
            <p className="help__text">
              İstanbul · Ankara · İzmir · Bursa · Konya — her biri bir hammaddede
              uzman; aynı malın fiyatı şehirden şehire değişir, kâr da bu farkta saklı.
            </p>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">
              ROLLER <span className="help__sec-note">sıralamada Sanayici + Tüccar</span>
            </h3>
            <div className="help__roles">
              <div className="help__role">
                <span className="help__badge help__badge--san">SAN</span>
                <span>Sanayici — fabrika kurar, ham → mamul üretir ve satar</span>
              </div>
              <div className="help__role">
                <span className="help__badge help__badge--tuc">TÜC</span>
                <span>Tüccar — ucuz şehirden alır, pahalı şehirde satar (kervanla)</span>
              </div>
              <div className="help__role help__role--dim">
                <span className="help__badge help__badge--ghost">+</span>
                <span>arka planda Çiftçi, Alıcı, Spekülatör ve Banka da işliyor</span>
              </div>
            </div>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">EKRANDAKİ PANELLER</h3>
            <div className="help__panels">
              {PANELS.map(([name, desc]) => (
                <div className="help__panel-row" key={name}>
                  <span className="help__panel-name">{name}</span>
                  <span className="help__panel-desc">{desc}</span>
                </div>
              ))}
            </div>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">SEZON</h3>
            <p className="help__text">
              Her sezon 90 tick sürer (~7,5 dk, 5 sn = 1 tick). Sezon bitince yeni
              tohumla (seed) sıfırdan yepyeni bir ekonomi başlar — sıralama, fiyatlar
              ve haberler baştan kurulur.
            </p>
          </section>
        </div>

        <div className="help__foot">
          <button className="help__cta" onClick={onClose}>
            anladım, izlemeye başla →
          </button>
        </div>
      </div>
    </div>
  );
}
