//! `MoneyWar` LAN multiplayer protokolü.
//!
//! Client ↔ server arasında giden mesajların tip tanımları. Wire format:
//! **postcard binary + 4-byte LE u32 length-prefix**. JSON denemiştik ama
//! `BTreeMap<(CityId, ProductKind), ...>` gibi tuple-keyed map'ler JSON
//! anahtarı zorunlu string kuralı yüzünden çalışmıyor. postcard tuple
//! anahtarları doğal kabul ediyor + ~3× daha kompakt + ~10× daha hızlı.
//!
//! ## Sözleşme
//!
//! - Server **tek otorite**. `advance_tick` orada çağrılır, broadcast eder.
//! - Client emirleri öneridir; server kabul eder ya da `CommandRejected` ile
//!   reddeder.
//! - Lockstep tick: server her tick'te biriken emirleri batch işler, sonra
//!   tüm peer'lara `TickAdvanced` yollar.
//! - Schema evolution: `Option<T>` + `#[serde(default)]` ile additive
//!   değişiklikler `PROTOCOL_VERSION` aynı kalır. Breaking değişiklik major
//!   bump → `Reject { reason: ProtocolMismatch }`.
//!
//! ## Frame format
//!
//! ```text
//! [len: u32 LE][payload: len bytes JSON]
//! ```
//!
//! `tokio_util::codec::LengthDelimitedCodec` ile çerçeveleme. JSON gövde
//! `ClientMessage` veya `ServerMessage` enum'unun `serde_json` çıktısı.

#![forbid(unsafe_code)]

use moneywar_domain::{
    CityId, Command, GameState, NewsTier, PlayerId, ProductKind, Role, RoomId, Tick,
};
use serde::{Deserialize, Serialize};

/// Protokol versiyonu — additive (yeni alan ekleme) minor değişikliklerde
/// aynı kalır; semantic break olduğunda artırılır. Server `Hello`'da gelen
/// versiyonu kontrol eder, uyuşmuyorsa `Reject` yollar.
pub const PROTOCOL_VERSION: u32 = 1;

/// Bir LAN odasında kabul edilen maksimum insan oyuncu sayısı. NPC'ler
/// `12 - human_count` kadar dolar (min 8 NPC).
pub const MAX_HUMAN_PLAYERS: u8 = 4;

/// Default tick periyodu (ms) — host lobide override edebilir.
pub const DEFAULT_TICK_MS: u64 = 300;

// ---------------------------------------------------------------------------
// Client → Server
// ---------------------------------------------------------------------------

/// Client'ın server'a gönderdiği mesajlar. postcard externally-tagged
/// enum encoding kullanır — variant index 1 byte (varint) + payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Bağlantı açıldıktan sonra ilk mesaj. Versiyon kontrolü ve isim sahipliği.
    Hello {
        protocol_version: u32,
        client_version: String,
        player_name: String,
    },

    /// Lobide rol seç (Sanayici/Tüccar). Sahiplenme server'da çakışmasız tutulur.
    SelectRole { role: Role },

    /// Lobide hazırım sinyali. Tüm oyuncular ready basınca host `GameStart`
    /// tetikler (ya da `tick_mode = auto` ise host'un ready'siyle başlar).
    Ready { ready: bool },

    /// Aktif oyunda emir gönder. Server bir sonraki tick batch'ine ekler.
    SubmitCommand { command: Command },

    /// Bağlantıyı koru. 1 Hz, server karşılığında `Pong` döner.
    Ping { nonce: u64 },

    /// Temiz çıkış — server diğer client'lara `PlayerLeft` yayınlar.
    Bye,
}

// ---------------------------------------------------------------------------
// Server → Client
// ---------------------------------------------------------------------------

/// Server'ın client'a gönderdiği mesajlar. Authoritative state akışı buradan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    /// `Hello`'ya başarılı yanıt. Player ID + room kimliği client'a iletilir.
    Welcome {
        protocol_version: u32,
        server_version: String,
        player_id: PlayerId,
        room_id: RoomId,
    },

    /// Bağlantı reddedildi (versiyon uyuşmazlığı, oda dolu, vb).
    Reject { reason: RejectReason },

    /// Lobi durumu — her değişiklikte yayınlanır (oyuncu giriş/çıkış,
    /// rol değiştirme, ready toggle).
    LobbyState {
        entries: Vec<LobbyEntry>,
        host: PlayerId,
    },

    /// Oyun başladı — initial state ve config tüm peer'lara aynı seed ile.
    GameStart {
        initial_state: Box<GameState>,
        tick_ms: u64,
    },

    /// Tick ilerledi. v0.1.x: full state broadcast (basit, ~5-25 KB).
    /// Sprint 4'te delta + hash check eklenecek.
    TickAdvanced {
        tick: Tick,
        state: Box<GameState>,
        /// İleride desync tespit için. v0.1.x'te `None` — Sprint 4'te
        /// `Some(blake3(state))`.
        #[serde(default)]
        state_hash: Option<String>,
    },

    /// Tek bir komut server tarafından kabul edildi/reddedildi (kullanıcı
    /// feedback'i için ayrı kanal — ana akıştan bağımsız).
    CommandRejected { command: Command, reason: String },

    /// Bir oyuncu odadan ayrıldı (disconnect / temiz çıkış).
    PlayerLeft { player_id: PlayerId, clean: bool },

    /// `Ping`'e cevap. Client RTT ölçer.
    Pong { nonce: u64 },
}

/// Lobide bir oyuncunun gözlenen durumu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LobbyEntry {
    pub player_id: PlayerId,
    pub player_name: String,
    pub role: Option<Role>,
    pub ready: bool,
    /// Tüccar bedava silver alır gibi rol-bazlı bonus için ileride genişler.
    #[serde(default)]
    pub news_tier: Option<NewsTier>,
}

/// `Reject` sebepleri — kullanıcıya gösterilebilir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RejectReason {
    ProtocolMismatch { expected: u32, got: u32 },
    RoomFull { capacity: u8 },
    NameTaken,
    GameAlreadyStarted,
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Wire format helpers
// ---------------------------------------------------------------------------

/// Wire format hataları — encode/decode başarısızlıkları.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("postcard encode hatası: {0}")]
    Encode(postcard::Error),
    #[error("postcard decode hatası: {0}")]
    Decode(postcard::Error),
}

/// Bir `ClientMessage`'ı postcard byte vektörüne çevir.
///
/// Framing (length-prefix) bu fonksiyonun **dışında** yapılır —
/// `tokio_util::codec::LengthDelimitedCodec` payload'a otomatik 4-byte
/// length ekler.
///
/// # Errors
/// `postcard::to_allocvec` başarısız olursa `WireError::Encode`.
///
/// # Examples
///
/// ```
/// use moneywar_net::{ClientMessage, decode_client, encode_client, PROTOCOL_VERSION};
///
/// let msg = ClientMessage::Hello {
///     protocol_version: PROTOCOL_VERSION,
///     client_version: "0.1.1".into(),
///     player_name: "Sen".into(),
/// };
/// let bytes = encode_client(&msg).unwrap();
/// let back = decode_client(&bytes).unwrap();
/// assert!(matches!(back, ClientMessage::Hello { .. }));
/// ```
pub fn encode_client(msg: &ClientMessage) -> Result<Vec<u8>, WireError> {
    postcard::to_allocvec(msg).map_err(WireError::Encode)
}

/// postcard byte slice'ından `ClientMessage` decode et.
///
/// # Errors
/// `postcard::from_bytes` başarısız olursa `WireError::Decode`.
pub fn decode_client(bytes: &[u8]) -> Result<ClientMessage, WireError> {
    postcard::from_bytes(bytes).map_err(WireError::Decode)
}

/// Bir `ServerMessage`'ı postcard byte vektörüne çevir.
///
/// # Errors
/// `postcard::to_allocvec` başarısız olursa `WireError::Encode`.
pub fn encode_server(msg: &ServerMessage) -> Result<Vec<u8>, WireError> {
    postcard::to_allocvec(msg).map_err(WireError::Encode)
}

/// postcard byte slice'ından `ServerMessage` decode et.
///
/// # Errors
/// `postcard::from_bytes` başarısız olursa `WireError::Decode`.
pub fn decode_server(bytes: &[u8]) -> Result<ServerMessage, WireError> {
    postcard::from_bytes(bytes).map_err(WireError::Decode)
}

// Compile-time guard: `Command` zaten serde derive'lıdır; protokol bunu
// aynen iletir. Eğer ileride `Command` non-serde olursa bu modül kırılır
// ve buradan haberdar oluruz.
const _: fn() = || {
    fn assert_serde<T: Serialize + for<'de> Deserialize<'de>>() {}
    assert_serde::<Command>();
    assert_serde::<GameState>();
    assert_serde::<CityId>();
    assert_serde::<ProductKind>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_hello_roundtrip() {
        let msg = ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            client_version: "0.1.1".into(),
            player_name: "Sen".into(),
        };
        let bytes = encode_client(&msg).unwrap();
        let decoded = decode_client(&bytes).unwrap();
        match decoded {
            ClientMessage::Hello {
                protocol_version,
                client_version,
                player_name,
            } => {
                assert_eq!(protocol_version, PROTOCOL_VERSION);
                assert_eq!(client_version, "0.1.1");
                assert_eq!(player_name, "Sen");
            }
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    #[test]
    fn server_reject_protocol_mismatch_roundtrip() {
        let msg = ServerMessage::Reject {
            reason: RejectReason::ProtocolMismatch {
                expected: 2,
                got: 1,
            },
        };
        let bytes = encode_server(&msg).unwrap();
        let back = decode_server(&bytes).unwrap();
        match back {
            ServerMessage::Reject {
                reason: RejectReason::ProtocolMismatch { expected, got },
            } => {
                assert_eq!(expected, 2);
                assert_eq!(got, 1);
            }
            other => panic!("expected Reject(ProtocolMismatch), got {other:?}"),
        }
    }

    #[test]
    fn submit_command_carries_domain_command() {
        let cmd = Command::BuildFactory {
            owner: PlayerId::new(1),
            city: CityId::Istanbul,
            product: ProductKind::Kumas,
        };
        let msg = ClientMessage::SubmitCommand { command: cmd };
        let bytes = encode_client(&msg).unwrap();
        let decoded = decode_client(&bytes).unwrap();
        match decoded {
            ClientMessage::SubmitCommand {
                command: Command::BuildFactory { city, product, .. },
            } => {
                assert_eq!(city, CityId::Istanbul);
                assert_eq!(product, ProductKind::Kumas);
            }
            other => panic!("expected SubmitCommand(BuildFactory), got {other:?}"),
        }
    }

    #[test]
    fn pong_roundtrip() {
        let msg = ServerMessage::Pong { nonce: 42 };
        let bytes = encode_server(&msg).unwrap();
        let back = decode_server(&bytes).unwrap();
        match back {
            ServerMessage::Pong { nonce } => assert_eq!(nonce, 42),
            other => panic!("expected Pong, got {other:?}"),
        }
    }

    #[test]
    fn lobby_entry_roundtrip() {
        let entry = LobbyEntry {
            player_id: PlayerId::new(7),
            player_name: "X".into(),
            role: None,
            ready: false,
            news_tier: None,
        };
        let bytes = postcard::to_allocvec(&entry).unwrap();
        let decoded: LobbyEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.player_id, PlayerId::new(7));
        assert_eq!(decoded.player_name, "X");
        assert!(decoded.role.is_none());
        assert!(decoded.news_tier.is_none());
    }
}
