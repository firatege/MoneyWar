//! Newtype ID tipleri.
//!
//! Her domain tipi için ayrı ID: `PlayerId`, `RoomId`, `OrderId` vb.
//! Newtype pattern compile-time'da karıştırmayı engeller —
//! `PlayerId::new(1) == RoomId::new(1)` derlenmez.
//!
//! Tüm ID'ler `u64` sarar, `#[serde(transparent)]` ile JSON'da düz sayı olarak görünür.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Yeni bir `Id` newtype'ı tanımlar. Her ID şu türlere sahiptir:
/// `Debug, Clone, Copy, Eq, Hash, Ord, Serialize, Deserialize, Display, From<u64>`.
macro_rules! define_id {
    ($name:ident, $prefix:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Ham `u64` değerden ID kurar.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Sarılı `u64` değeri döndürür.
            #[must_use]
            pub const fn value(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}-{}", $prefix, self.0)
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }
    };
}

define_id!(PlayerId, "PLY");
define_id!(RoomId, "ROOM");
define_id!(OrderId, "ORD");
define_id!(ContractId, "CTR");
define_id!(FactoryId, "FAC");
define_id!(CaravanId, "CRV");
define_id!(NewsId, "NEWS");
define_id!(EventId, "EVT");
define_id!(LoanId, "LOAN");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_construct_from_u64() {
        let p = PlayerId::new(42);
        assert_eq!(p.value(), 42);
    }

    #[test]
    fn ids_display_with_prefix() {
        assert_eq!(PlayerId::new(42).to_string(), "PLY-42");
        assert_eq!(RoomId::new(7).to_string(), "ROOM-7");
        assert_eq!(OrderId::new(1).to_string(), "ORD-1");
        assert_eq!(ContractId::new(99).to_string(), "CTR-99");
        assert_eq!(FactoryId::new(3).to_string(), "FAC-3");
        assert_eq!(CaravanId::new(5).to_string(), "CRV-5");
        assert_eq!(NewsId::new(100).to_string(), "NEWS-100");
        assert_eq!(EventId::new(200).to_string(), "EVT-200");
        assert_eq!(LoanId::new(8).to_string(), "LOAN-8");
    }

    #[test]
    fn ids_serialize_as_transparent_u64() {
        let p = PlayerId::new(42);
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(json, "42");

        let back: PlayerId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, p);
    }

    #[test]
    fn ids_order_for_btreemap_keys() {
        let a = PlayerId::new(1);
        let b = PlayerId::new(2);
        assert!(a < b);
    }

    #[test]
    fn ids_from_u64_conversion() {
        let p: PlayerId = 42u64.into();
        assert_eq!(p.value(), 42);
    }

    #[test]
    fn ids_hash_consistently() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        PlayerId::new(5).hash(&mut h1);
        PlayerId::new(5).hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }
}
