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

/** Oyunun temel kuralları — ekranda gördüğün her şey bunlardan çıkıyor. */
const RULES: Array<[string, string]> = [
  [
    "Tick",
    "Oyun turlar hâlinde ilerler. Her tick'te herkes aynı anda karar verir, emirler tek seferde eşleşir, üretim bir adım ilerler. Kimse sıra kapmaz — hız değil karar önemlidir.",
  ],
  [
    "Pazar",
    "Her şehirde her mal için ayrı bir pazar var. Alım ve satım emirleri tick sonunda toplu eşleşir; oluşan fiyat o malın o şehirdeki yeni fiyatıdır. Emirlerin ömrü sınırlıdır, eşleşmezse düşer.",
  ],
  [
    "Fabrika",
    "Girdi alır, bir süre sonra mamul verir. Seviye büyüdükçe parti büyür ve hızlanır. Kadrosu eksikse aynı oranda az üretir; girdisi tükenirse bant durur — “atıl” dediğimiz durum budur.",
  ],
  [
    "İşçi",
    "Ortak bir işgücü havuzu var. Her fabrika kadro tutar, karşılığında ücret öder ve o ücret Alıcılara gider; onlar da mal alır. Para böylece döner — kimse boşluktan para basmaz.",
  ],
  [
    "Kervan",
    "Mal şehirler arası kendiliğinden ışınlanmaz. Tüccar kervanla taşır, yolculuk tick alır. Haritadaki yolun üstünde ilerleyen nokta budur.",
  ],
  [
    "Sözleşme",
    "Firmalar ileri tarihli teslim sözü verebilir. Söz verilen mal anında kilitlenir; sözünü tutmayan tarafın güveni düşer ve bir daha kimse onunla kolay iş yapmaz.",
  ],
  [
    "İflas",
    "Borcunu ve giderlerini karşılayamayan firma batar. Fabrikaları elden çıkar, akışta “İFLAS” kartı olarak görünür.",
  ],
];

/** İzleyicinin asıl takip ettiği şey: firmaların birbirine yaptıkları. */
const INTRIGUE: Array<[string, string, string]> = [
  ["👑", "Tekel", "Bir pazardaki satışın ezici çoğunluğu tek firmada toplandı. Fiyatı artık o belirler."],
  ["✂️", "Fiyat kırma", "Rakibin altına girip onu pazardan atmaya çalışmak. Kısa vadede kâr yakar, uzun vadede pazar kazandırır."],
  ["🔒", "Tedarik boğma", "Rakibin fabrikasının ihtiyacı olan girdiyi toplayıp piyasadan çekmek. Rakip üretemez hâle gelir."],
  ["⚔️", "Fiyat savaşı", "Karşılıklı kırma. Biri pes edene kadar sürer; kazanan pazarı alır."],
  ["🤝", "Kartel", "İki firma fiyatı yüksek tutmakta anlaşır. Kârlıdır ama kırılgandır."],
  ["🗡️", "İhanet", "Kartel ortağı anlaşmayı bozup tek başına kırar."],
  ["🔥", "Kin", "Zarar gören firma zarar vereni hatırlar; sonraki kararlarında ona karşı sert davranır."],
];

/** Ekrandaki bölümler — soldan sağa, üstten alta. */
const PANELS: Array<[string, string]> = [
  [
    "ROLLER",
    "her rolün kişi başı kârı. Çubuk sıfırdan iki yana açılır: sağ kâr, sol zarar. Ekonominin kimin lehine işlediği burada görünür.",
  ],
  [
    "SIRALAMA",
    "firmalar kâra göre dizili. Üstteki düğmelerle role göre süz (şirketler / tüccar / çiftçi / hepsi); bir satıra tıkla, firma sayfası açılsın.",
  ],
  [
    "ŞEHİR AĞI",
    "beş şehir, aralarındaki on yolun hepsi. Dairedeki sayı fabrika adedi, çevresindeki halka kaçının çalıştığı; yol kalınlığı üstündeki kervan sayısı, kırmızı nokta o şehirde tekel var demek.",
  ],
  [
    "AKIŞ",
    "üç katman. Üstte dönüm noktaları (tekel, iflas, kartel — nadir ve önemli), ortada süregelen çekişme, altta fon: sık tekrar eden olaylar tek tek değil sayılarak.",
  ],
  [
    "ALT ŞERİT",
    "ekonominin sağlığı: para arzı ve fiyat endeksi, fabrikalar (her kare bir fabrika), istihdam, servet dağılımı (Gini) ve üretim zincirinin hangi katmanı tıkalı.",
  ],
];

/** Üç kademeli inceleme — kullanıcının "ne aradığını bulması" bu akışta. */
const DRILL: Array<[string, string, string]> = [
  [
    "1",
    "Şehre tıkla",
    "oradaki fabrikalar ve sahipleri, kim kimle iş yapıyor, ne üretiliyor, ne tutuluyor, fiyat seyri ve son işlemler.",
  ],
  [
    "2",
    "Firmaya tıkla",
    "kâr/zarar seyri, fabrikaları nerede ve hangi durumda, envanteri, ticaret ortakları ve onlara duyduğu güven, neyi alıp neyi sattığı.",
  ],
  [
    "3",
    "Fabrikaya tıkla",
    "kadrosu, girdilerinin durumu (bandı ne durdurmuş), birim maliyet ile piyasa fiyatı arasındaki marj, üretim geçmişi ve bu üründen son satışları.",
  ],
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
            batırır. Sen izlersin. Ekran soldan sağa üç soruya bakar:{" "}
            <b>kim kazanıyor</b>, <b>nerede oluyor</b>, <b>ne oluyor</b> — altta da
            ekonominin genel sağlığı.
          </p>

          <section className="help__sec">
            <h3 className="help__sec-title">
              NASIL İŞLİYOR <span className="help__sec-note">oyunun kuralları</span>
            </h3>
            <div className="help__panels">
              {RULES.map(([name, desc]) => (
                <div className="help__panel-row" key={name}>
                  <span className="help__panel-name">{name}</span>
                  <span className="help__panel-desc">{desc}</span>
                </div>
              ))}
            </div>
          </section>

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
              Katman yükseldikçe parti küçülür: bir Un fabrikası birden çok Ekmek
              fabrikasını besleyebilsin diye. Üst katman az ve değerli üretir —
              ama tek bir girdisi eksik olsa bant durur, kırılganlığı buradan gelir.
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
              ROLLER <span className="help__sec-note">hepsi sıralamada, filtreyle süz</span>
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
                <span>
                  Çiftçi hammadde üretir · Alıcı tüketir · Spekülatör stok tutar ·
                  Banka kredi verir
                </span>
              </div>
            </div>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">EKRANDAKİ BÖLÜMLER</h3>
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
            <h3 className="help__sec-title">
              DERİNE İN{" "}
              <span className="help__sec-note">şehir → firma → fabrika</span>
            </h3>
            <div className="help__drill">
              {DRILL.map(([n, title, desc]) => (
                <div className="help__step" key={n}>
                  <span className="help__step-n">{n}</span>
                  <span className="help__step-body">
                    <b className="help__step-title">{title}</b>
                    <span className="help__step-desc">{desc}</span>
                  </span>
                </div>
              ))}
            </div>
            <p className="help__note">
              <kbd className="help__kbd">Esc</kbd> her zaman bir adım geri, haritaya
              döndürür.
            </p>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">
              ENTRİKA <span className="help__sec-note">akışta bunları görürsün</span>
            </h3>
            <div className="help__intrigue">
              {INTRIGUE.map(([icon, name, desc]) => (
                <div className="help__intrigue-row" key={name}>
                  <span className="help__intrigue-icon" aria-hidden="true">
                    {icon}
                  </span>
                  <span className="help__intrigue-body">
                    <b className="help__intrigue-name">{name}</b>
                    <span className="help__intrigue-desc">{desc}</span>
                  </span>
                </div>
              ))}
            </div>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">
              SAYILARI OKUMAK <span className="help__sec-note">sık karşına çıkanlar</span>
            </h3>
            <div className="help__panels">
              <div className="help__panel-row">
                <span className="help__panel-name">PnL</span>
                <span className="help__panel-desc">
                  Kâr/zarar. Sezon başındaki toplam varlığa (nakit + mal) göre bugünkü
                  fark. Sıfır = başladığı yerde.
                </span>
              </div>
              <div className="help__panel-row">
                <span className="help__panel-name">Gini</span>
                <span className="help__panel-desc">
                  Dağılımın eşitsizliği. 0 herkes eşit, 1 her şey tek elde. Servette
                  0,3 altı dengeli, 0,5 üstü tek elde toplanıyor demek.
                </span>
              </div>
              <div className="help__panel-row">
                <span className="help__panel-name">Endeks</span>
                <span className="help__panel-desc">
                  Tüm fiyatların baz fiyata oranı ×100. 100 üstü piyasa pahalı, altı
                  ucuz.
                </span>
              </div>
              <div className="help__panel-row">
                <span className="help__panel-name">Marj</span>
                <span className="help__panel-desc">
                  Bir mamulün piyasa fiyatının, tarif maliyetini ne kadar aştığı.
                  Sıfıra yaklaşan ürünü üretmek anlamsızlaşır.
                </span>
              </div>
              <div className="help__panel-row">
                <span className="help__panel-name">Güven</span>
                <span className="help__panel-desc">
                  İki firma arasındaki ticaret geçmişinden çıkar. Yüksek güven daha
                  kolay sözleşme, düşük güven kapanan kapı demek.
                </span>
              </div>
            </div>
          </section>

          <section className="help__sec">
            <h3 className="help__sec-title">SEZON</h3>
            <p className="help__text">
              Her sezon 350 tick sürer (~17,5 dk, 3 sn = 1 tick). Üstteki çubuk ne
              kadar kaldığını gösterir. Sezon bitince yeni tohumla (seed) sıfırdan
              yepyeni bir ekonomi başlar — şehirlerin uzmanlığı, fiyatlar ve firmalar
              baştan kurulur; eski sezonlar “geçmiş” düğmesinde durur.
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
