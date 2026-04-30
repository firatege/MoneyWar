# Bidirectional Kontrat Sistemi — Implementation Plan

> **Status:** ⏸ Planlandı, henüz başlanmadı.
> **Hedef sürüm:** v0.3.0 (breaking domain change).
> **Tahmini efor:** 13-19 saat (4-5 commit'lik iş).

## Niyet

Kontrat mekaniğini "yan feature"dan **merkezi oyun mekaniğine** dönüştür. Şu an
sadece satıcı kontrat öneriyor; alıcı da "şu malı şu fiyatta isterim, kim
teslim eder?" çağrısı (RFQ — Request For Quote) yapabilsin. NPC AI iki yönde
de aday üretsin. UI ve sezon-akışı kontratları öne çıkarsın.

## Mevcut Sistem Haritası

| Dosya | Boyut | Değişiklik etki |
|---|---|---|
| `domain/src/contract.rs` | ~400 satır + 30 test | **Yüksek** — struct ve API rename |
| `engine/src/contracts.rs` | ~? satır + integration test | **Yüksek** — propose/accept/fulfill simetrik dallanma |
| `npc/src/dss/contract.rs` | ~200 satır | **Orta** — buyer-request adayı eklenir |
| `cli/src/main.rs` (kontrat overlay + Offer wizard) | ~500 satır | **Orta** — yeni `o` wizard + 2 sekmeli overlay |
| `engine/src/report.rs` (LogEvent fields) | ~100 satır | **Düşük** — field rename |
| Testler (8+ test dosyası kontrat referans veriyor) | — | **Orta** — proposer/side ile güncelle |

Toplam etkilenen yer: **~1500 satır kod + 30+ test**.

## Önerilen Mimari

### Domain modeli

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractSide {
    /// "Bu malı satıyorum, alıcı arıyorum" — mevcut davranış
    SellerOffer,
    /// "Bu malı almak istiyorum, satıcı arıyorum" — yeni
    BuyerRequest,
}

pub struct ContractProposal {
    pub proposer: PlayerId,           // eskiden 'seller'
    pub side: ContractSide,           // YENİ
    pub listing: ListingKind,         // korunur
    pub product: ProductKind,
    pub quantity: u32,
    pub unit_price: Money,
    pub delivery_city: CityId,
    pub delivery_tick: Tick,
    pub proposer_deposit: Money,      // eskiden 'seller_deposit'
    pub counterparty_deposit: Money,  // eskiden 'buyer_deposit'
}

pub struct Contract {
    // Aynı yeniden adlandırma + helper'lar:
    pub fn seller(&self) -> PlayerId { /* side'a göre proposer ya da counterparty */ }
    pub fn buyer(&self) -> Option<PlayerId> { /* side'a göre */ }
}
```

`Contract::seller()` / `buyer()` helper'ları sayesinde fulfill akışı (mal seller→buyer,
para buyer→seller) **tek kod yolu** olur. Dış dünya yine "seller/buyer" perspektifiyle
çalışır, sadece propose anında side belirleyici.

### Engine simetrisi

| Adım | SellerOffer (mevcut) | BuyerRequest (yeni) |
|---|---|---|
| Propose ücreti | seller_deposit (proposer'dan çek) | buyer_deposit (proposer'dan çek) |
| Accept ücreti | buyer_deposit (counterparty'den çek) | seller_deposit (counterparty'den çek) |
| Stok lock (proposal) | seller'dan değil — fulfill anında check | aynı |
| Fulfill | mal: seller→buyer, para: buyer→seller | aynı |
| Breach (seller stoksuz) | seller cezası → buyer'a deposit + bonus | aynı |
| Breach (buyer cashsız) | buyer cezası → seller | aynı |

Yani fulfill/breach **side'dan bağımsız** — sadece propose/accept aşamasında
deposit yönü farklı.

## Implementation Phases

### Phase 1 — Domain Refactor 🔴 (3-4 saat)
- `ContractSide` enum ekle.
- `ContractProposal` field rename: `seller` → `proposer`, `seller_deposit` →
  `proposer_deposit`, `buyer_deposit` → `counterparty_deposit`. Yeni `side` field.
- `Contract` aynı rename + `seller()`/`buyer()` helper.
- Validation: BuyerRequest için propose anında stok kontrolü yapma. Cash kontrolü
  proposer/buyer'dan.
- 30+ existing test güncelle (yeni field isimleri).

**Etkilenen yerler:** contract.rs, command.rs (`Command::ProposeContract`),
report.rs (LogEvent fields), tüm `c.seller`/`c.accepted_by` referansları.

**Risk:** Save formatı bozulur (serde rename). Kabul edildi: v0.3'te kırıcı,
save/load henüz yok.

### Phase 2 — Engine Refactor 🔴 (2-3 saat)
- `propose_contract_impl`: side'a göre proposer'dan doğru deposit'i çek.
- `accept_contract_impl`: side'a göre counterparty'den deposit + side'a göre
  stok/cash kontrolü.
- `fulfill_contract_impl`: side'dan bağımsız (helper kullan).
- `breach_contract_impl`: aynı.
- `cancel_contract_impl`: proposer iptali — deposit iade.

**Etkilenen testler:** contract_flow.rs (4 test), engine unit testleri.

### Phase 3 — NPC AI Bidirectional 🟡 (2-3 saat)

Yeni adaylar:
- **Sanayici (BuyerRequest):** "Fabrika ham maddesi için 100 Pamuk @ 5.50,
  t10 teslim İstanbul." Stok < eşik ise.
- **Esnaf (BuyerRequest):** "Dükkan stoğu için 80 Kumaş @ 18, t8 teslim
  Ankara." Mamul stoğu < 100 ise.
- **Tüccar (BuyerRequest):** Arbitraj fırsatı varsa hedef şehirde alış kontratı.
- **Tüccar (SellerOffer):** Mevcut korunur.

**Accept tarafı:** Zaten `accept_contract_candidates` her iki yönü destekler;
side'a göre cash/stok pre-flight değişir.

**NPC bias oranları (öneri):**
- Sanayici: BuyerRequest %70 / SellerOffer %30
- Tüccar: %50/%50
- Esnaf: %30/%70 (asıl iş satış)
- Spekülatör: %50/%50

**Risk:** NPC her tick hem buyer hem seller candidate çıkarırsa kontrat sayısı
şişer. **Çözüm:** propose rate-limit (her NPC tick başı max 1 kontrat).

### Phase 4 — CLI UI 🟡 (3-4 saat)

**Yeni `o` tuşu:** "alış kontrat öner" wizard (Order/Offer simetriği).
- Schema: Product → City → Quantity → PriceLira → DeliveryTick.
- Side=BuyerRequest sabit.

**`y` overlay 2 sekmeli:**
- Sekme 1: 📤 Sat (SellerOffer'lar)
- Sekme 2: 📥 Al (BuyerRequest'ler)
- ←/→ Tab ile geçiş (r overlay ve wizard ile aynı pattern).
- Her sekmede "[N] yeni öner" kısayolu.

**Status entegrasyonu:** Header'da `📜 5/3` (5 aktif kontrat / 3 bana yönelik).

**Sezon başı ipucu:** Tick 1 sonrası status: "Tip: Ham madde gerekli mi? `o`
ile alış kontratı önerebilirsin."

### Phase 5 — Skor Breakdown + Söylenti 🟢 (1-2 saat)
- `ScoreBreakdown` struct'ına `contract_pnl: Money` ekle (UI bilgi katmanı,
  formül değişmez — escrow zaten ana formülde).
- Leaderboard satır altında küçük renkli özet: "📜 +850₺ kontrat".
- Söylenti akışında kontrat olayları öne: "🎯 Naime Hanım 200 Pamuk alış
  çağrısı açtı — t12'ye kadar".

### Phase 6 — Test 🟡 (2-3 saat)
- BuyerRequest happy path: propose → seller accept → t arrival fulfill.
- BuyerRequest breach (seller stok yetmez): proposer (buyer) deposit'ini
  geri alır + counterparty (seller) deposit'i kaybeder.
- Money conservation BuyerRequest yolunda da geçerli.
- NPC bidirectional smoke: 100 tick sonra her iki yönde de kontrat sayısı > 0.
- Mevcut SellerOffer testleri side=SellerOffer ile uyumlu kalır.

## Açık Sorular (kullanıcıdan onay bekleniyor)

1. **`o` tuşu mu, mevcut Offer wizard'ına side seçimi mi?**
   - (a) Yeni `o` tuşu = "alış kontrat öner" — symmetric. **[önerim]**
   - (b) Mevcut Offer wizard'ında ilk adım "Sat mı Al mı?".

2. **NPC bias oranları yukarıdaki öneri ile başlanacak mı?**
   - Sanayici %70/%30, Tüccar %50/%50, Esnaf %30/%70, Spekülatör %50/%50.

3. **Skor breakdown nasıl?**
   - (a) Mevcut formül escrow ile zaten yansıtıyor.
   - (b) Sezon-içi kontrat **gerçekleşmiş** P&L'i ayrı UI satırı (formül değişmez). **[önerim]**

4. **Save formatı kırılır mı?** Save/load henüz yok, kabul.

5. **Tam rename mı, adapter mı?** Önerim: **tam rename** (temiz, teknik borç bırakmaz).

## Risk Listesi

- 🔴 **Domain rename zinciri** — 30+ ref. Test/helper unutulursa silent break.
  Mitigation: aşamalı search-replace + sık `cargo check`.
- 🟡 **NPC kontrat sayısı patlaması** — bidirectional → 2x potansiyel.
  Mitigation: tick başı max 1 propose, side bias.
- 🟡 **Buyer-side stok pre-flight yok** — counterparty kabul anında stok
  yetmezse instant breach. Mitigation: accept anında UI uyarısı.
- 🟡 **CLI overlay 2 sekme + yeni wizard** — ergonomi. Mitigation: legend net.
- 🟢 **Backwards compat** — save format yok.

## Karmaşıklık ve Faz Sıralaması

| Faz | Efor | Risk |
|---|---|---|
| 1. Domain refactor | 3-4 saat | 🔴 |
| 2. Engine refactor | 2-3 saat | 🟡 |
| 3. NPC AI | 2-3 saat | 🟡 |
| 4. CLI UI | 3-4 saat | 🟡 |
| 5. Skor/söylenti | 1-2 saat | 🟢 |
| 6. Test | 2-3 saat | 🟡 |
| **Toplam** | **13-19 saat** | **HIGH** |

Faz 1-2 birlikte (domain+engine birlikte build olur), sonra 3-4-5-6 ayrı commit'ler.

## Sonraki Adım

Açık sorulara cevap → onay → Faz 1'den başlama.
