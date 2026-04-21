//! Deterministik tick RNG'si.
//!
//! Seed = `hash(room_id, tick)`. Aynı (oda, tick) → aynı `ChaCha8Rng` stream'i.
//! Replay + property testler için şart: motor rastgelelik kullandığında bu
//! RNG'yi alır, böylece `advance_tick` saflığı korunur.
//!
//! `HashMap` veya `DefaultHasher` **kullanılmaz** — bunlar process başına
//! farklı seed üretir, determinism kırılır. Bunun yerine `SplitMix64` tarzı
//! sabit bit karma + `u64`'ten 32-byte seed genişletme yapıyoruz.

use moneywar_domain::{RoomId, Tick};
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::SeedableRng;

/// `(room_id, tick)` → 32-byte deterministik seed.
///
/// İki `u64`'ü `SplitMix64` ile karıştırıp 4 blok halinde 32 byte'a genişletir.
/// Aynı input her zaman aynı output — process/derleme bağımsız.
#[must_use]
pub fn seed_for(room_id: RoomId, tick: Tick) -> [u8; 32] {
    let mixed = mix64(room_id.value(), u64::from(tick.value()));
    let a = splitmix64(mixed);
    let b = splitmix64(a);
    let c = splitmix64(b);
    let d = splitmix64(c);

    let mut out = [0u8; 32];
    out[0..8].copy_from_slice(&a.to_le_bytes());
    out[8..16].copy_from_slice(&b.to_le_bytes());
    out[16..24].copy_from_slice(&c.to_le_bytes());
    out[24..32].copy_from_slice(&d.to_le_bytes());
    out
}

/// `(room_id, tick)` → kullanıma hazır `ChaCha8Rng`.
#[must_use]
pub fn rng_for(room_id: RoomId, tick: Tick) -> ChaCha8Rng {
    ChaCha8Rng::from_seed(seed_for(room_id, tick))
}

/// `SplitMix64` — klasik deterministik bit karma. Sabit sabit değerler
/// `Sebastiano Vigna`'nın referans implementasyonundan.
const fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// İki `u64`'ü simetriksiz karıştır — rotl + xor, sıra önemli.
const fn mix64(a: u64, b: u64) -> u64 {
    let x = a.wrapping_mul(0xC6A4_A793_5BD1_E995);
    let y = b.rotate_left(27).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    splitmix64(x ^ y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    #[test]
    fn seed_is_deterministic_for_same_input() {
        let a = seed_for(RoomId::new(1), Tick::new(5));
        let b = seed_for(RoomId::new(1), Tick::new(5));
        assert_eq!(a, b);
    }

    #[test]
    fn seed_differs_for_different_rooms() {
        let a = seed_for(RoomId::new(1), Tick::new(5));
        let b = seed_for(RoomId::new(2), Tick::new(5));
        assert_ne!(a, b);
    }

    #[test]
    fn seed_differs_for_different_ticks() {
        let a = seed_for(RoomId::new(1), Tick::new(5));
        let b = seed_for(RoomId::new(1), Tick::new(6));
        assert_ne!(a, b);
    }

    #[test]
    fn rng_produces_same_stream_for_same_input() {
        let mut r1 = rng_for(RoomId::new(42), Tick::new(10));
        let mut r2 = rng_for(RoomId::new(42), Tick::new(10));
        for _ in 0..16 {
            assert_eq!(r1.next_u64(), r2.next_u64());
        }
    }

    #[test]
    fn rng_diverges_for_different_input() {
        let mut r1 = rng_for(RoomId::new(1), Tick::new(10));
        let mut r2 = rng_for(RoomId::new(1), Tick::new(11));
        // İlk 4 çekim içinde en az biri farklı olmalı.
        let diverged = (0..4).any(|_| r1.next_u64() != r2.next_u64());
        assert!(diverged);
    }

    #[test]
    fn seed_is_32_bytes_and_not_all_zero() {
        let s = seed_for(RoomId::new(1), Tick::ZERO);
        assert_eq!(s.len(), 32);
        assert!(s.iter().any(|&b| b != 0));
    }
}
