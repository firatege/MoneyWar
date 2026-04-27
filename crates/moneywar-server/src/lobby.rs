//! Server-side lobby state — Sprint 2.
//!
//! Bağlanan oyuncuları odada tutar, rol seçimi + Ready toggle akışını
//! yönetir, tüm oyuncular hazır olduğunda `GameStart` broadcast tetikler.
//!
//! Per-connection broadcast: her oyuncu için bir `mpsc::Sender<ServerMessage>`
//! tutuyoruz; lobby state değiştikçe (rol seçimi, Ready, oyuncu giriş/çıkış)
//! tüm slot'lara `LobbyState` mesajı yayınlıyoruz.
//!
//! Sprint 2'de tek room (host=ilk gelen). Sprint 4+ multi-room genişlemesi.

use moneywar_domain::{PlayerId, Role, RoomId};
use moneywar_net::{LobbyEntry, MAX_HUMAN_PLAYERS, ServerMessage};
use tokio::sync::mpsc;

/// Bir bağlı oyuncunun lobi durumu + dışarı yazma kanalı.
pub struct LobbySlot {
    pub name: String,
    pub role: Option<Role>,
    pub ready: bool,
    /// `tx` üzerinden bu oyuncuya server mesajı yollanır. Connection task'ı
    /// `rx`'i dinleyip framed sink'e yazar.
    pub tx: mpsc::Sender<ServerMessage>,
}

/// Oda durumu — host + slot map. `room_id` server boyunca sabit (Sprint 2).
pub struct Lobby {
    pub room_id: RoomId,
    pub host: Option<PlayerId>,
    pub slots: std::collections::BTreeMap<PlayerId, LobbySlot>,
    /// Oyun başladıysa true — yeni gelenleri reddet. Sprint 3'te oyun
    /// devam ederken late-join açılacak (snapshot ile).
    pub game_started: bool,
}

impl Lobby {
    pub fn new(room_id: RoomId) -> Self {
        Self {
            room_id,
            host: None,
            slots: std::collections::BTreeMap::new(),
            game_started: false,
        }
    }

    /// Yeni oyuncu eklendi. İlk gelen host olur. Lobi dolu ya da oyun başladıysa
    /// `Err` döner — caller `Reject` mesajını yollamalı.
    pub fn add_player(
        &mut self,
        player_id: PlayerId,
        name: String,
        tx: mpsc::Sender<ServerMessage>,
    ) -> Result<(), JoinError> {
        if self.game_started {
            return Err(JoinError::GameAlreadyStarted);
        }
        if self.slots.len() >= MAX_HUMAN_PLAYERS as usize {
            return Err(JoinError::RoomFull);
        }
        if self.slots.values().any(|s| s.name == name) {
            return Err(JoinError::NameTaken);
        }
        if self.host.is_none() {
            self.host = Some(player_id);
        }
        self.slots.insert(
            player_id,
            LobbySlot {
                name,
                role: None,
                ready: false,
                tx,
            },
        );
        Ok(())
    }

    /// Oyuncu odadan ayrıldı. Host ayrılırsa diğer oyunculardan ilki host olur.
    /// Oda boş kalırsa `host = None`, `game_started = false` (yeni oda olur).
    pub fn remove_player(&mut self, player_id: PlayerId) {
        self.slots.remove(&player_id);
        if self.host == Some(player_id) {
            self.host = self.slots.keys().next().copied();
        }
        if self.slots.is_empty() {
            self.host = None;
            self.game_started = false;
        }
    }

    /// Bir oyuncu rol seçti. Aynı rol başkası tarafından alınmışsa kabul ederiz —
    /// MoneyWar'da her oyuncu istediği rolü seçebilir (Sanayici/Tüccar
    /// karışık olabilir, rekabet bunun üzerinden). Ready'yi sıfırla.
    pub fn select_role(&mut self, player_id: PlayerId, role: Role) {
        if let Some(slot) = self.slots.get_mut(&player_id) {
            slot.role = Some(role);
            slot.ready = false; // rol değişince yeniden onay gerek
        }
    }

    /// Ready toggle. Rol seçilmemişse no-op (sessiz reddet).
    pub fn set_ready(&mut self, player_id: PlayerId, ready: bool) {
        if let Some(slot) = self.slots.get_mut(&player_id) {
            if slot.role.is_some() {
                slot.ready = ready;
            }
        }
    }

    /// Tüm slot'lar (en az 2) ready ve rolü var mı?
    pub fn all_ready(&self) -> bool {
        self.slots.len() >= 2 && self.slots.values().all(|s| s.ready && s.role.is_some())
    }

    /// Mevcut snapshot'ı `LobbyState` mesajına çevir.
    pub fn snapshot(&self) -> ServerMessage {
        let entries: Vec<LobbyEntry> = self
            .slots
            .iter()
            .map(|(id, slot)| LobbyEntry {
                player_id: *id,
                player_name: slot.name.clone(),
                role: slot.role,
                ready: slot.ready,
                news_tier: None,
            })
            .collect();
        ServerMessage::LobbyState {
            entries,
            host: self.host.unwrap_or_else(|| PlayerId::new(0)),
        }
    }

    /// Tüm bağlı oyunculara verilen mesajı yolla. Kanal kapalıysa o slot'u
    /// temizleme caller'ın görevi (connection task'ı zaten drop'ta yapıyor).
    pub async fn broadcast(&self, msg: &ServerMessage) {
        for slot in self.slots.values() {
            // Try-send değil await — backpressure mantıklı (lobby mesajları az).
            let _ = slot.tx.send(msg.clone()).await;
        }
    }
}

/// Oda katılım hatası.
#[derive(Debug, Clone, Copy)]
pub enum JoinError {
    RoomFull,
    NameTaken,
    GameAlreadyStarted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{PlayerId, Role, RoomId};

    fn dummy_tx() -> mpsc::Sender<ServerMessage> {
        let (tx, _rx) = mpsc::channel(8);
        tx
    }

    #[test]
    fn first_player_becomes_host() {
        let mut lobby = Lobby::new(RoomId::new(1));
        lobby
            .add_player(PlayerId::new(1), "Ali".into(), dummy_tx())
            .unwrap();
        assert_eq!(lobby.host, Some(PlayerId::new(1)));
    }

    #[test]
    fn host_migrates_when_host_leaves() {
        let mut lobby = Lobby::new(RoomId::new(1));
        lobby
            .add_player(PlayerId::new(1), "Ali".into(), dummy_tx())
            .unwrap();
        lobby
            .add_player(PlayerId::new(2), "Veli".into(), dummy_tx())
            .unwrap();
        lobby.remove_player(PlayerId::new(1));
        assert_eq!(lobby.host, Some(PlayerId::new(2)));
    }

    #[test]
    fn duplicate_name_rejected() {
        let mut lobby = Lobby::new(RoomId::new(1));
        lobby
            .add_player(PlayerId::new(1), "Ali".into(), dummy_tx())
            .unwrap();
        let err = lobby
            .add_player(PlayerId::new(2), "Ali".into(), dummy_tx())
            .unwrap_err();
        assert!(matches!(err, JoinError::NameTaken));
    }

    #[test]
    fn role_select_resets_ready() {
        let mut lobby = Lobby::new(RoomId::new(1));
        lobby
            .add_player(PlayerId::new(1), "Ali".into(), dummy_tx())
            .unwrap();
        lobby.select_role(PlayerId::new(1), Role::Tuccar);
        lobby.set_ready(PlayerId::new(1), true);
        assert!(lobby.slots[&PlayerId::new(1)].ready);
        // Rol değiştirince ready sıfırlanır.
        lobby.select_role(PlayerId::new(1), Role::Sanayici);
        assert!(!lobby.slots[&PlayerId::new(1)].ready);
    }

    #[test]
    fn all_ready_requires_two_players_with_roles() {
        let mut lobby = Lobby::new(RoomId::new(1));
        lobby
            .add_player(PlayerId::new(1), "Ali".into(), dummy_tx())
            .unwrap();
        lobby.select_role(PlayerId::new(1), Role::Tuccar);
        lobby.set_ready(PlayerId::new(1), true);
        // Tek oyuncu → all_ready false
        assert!(!lobby.all_ready());
        lobby
            .add_player(PlayerId::new(2), "Veli".into(), dummy_tx())
            .unwrap();
        lobby.select_role(PlayerId::new(2), Role::Sanayici);
        lobby.set_ready(PlayerId::new(2), true);
        assert!(lobby.all_ready());
    }
}
