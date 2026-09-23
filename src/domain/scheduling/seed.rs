//! Deterministic randomness: seed derivation and the generator behind the scheduler.

use std::hash::Hasher;

use fnv::FnvHasher;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// The generator used by the scheduler (ChaCha8: value-stable across platforms and versions).
pub type ScheduleRng = ChaCha8Rng;

pub fn rng_from_seed(seed: u64) -> ScheduleRng {
    ScheduleRng::seed_from_u64(seed)
}

/// FNV-1a over a list of byte strings (stable seed derivation); parts cannot run together.
pub fn derive_seed(parts: &[&[u8]]) -> u64 {
    let mut h = FnvHasher::default();
    for p in parts {
        h.write(p);
        h.write_u8(0xff);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use rand::RngExt;

    use super::*;

    #[test]
    fn rng_is_deterministic_and_bounded() {
        let (mut a, mut b) = (rng_from_seed(42), rng_from_seed(42));
        for _ in 0..100 {
            assert_eq!(a.random_range(0..u64::MAX), b.random_range(0..u64::MAX));
        }
        let mut r = rng_from_seed(1);
        for _ in 0..1000 {
            assert!(r.random_range(0..7u64) < 7);
            assert!((0.0..1.0).contains(&r.random::<f64>()));
        }
    }

    #[test]
    fn seed_derivation_is_stable_and_unambiguous() {
        assert_eq!(derive_seed(&[b"a"]), derive_seed(&[b"a"]));
        assert_ne!(derive_seed(&[b"a", b"b"]), derive_seed(&[b"ab"]));
    }

    /// Golden values: changing the generator or the hash would reshuffle every schedule.
    #[test]
    fn generator_and_seed_derivation_are_pinned() {
        let mut r = rng_from_seed(42);
        let drawn: Vec<u64> = (0..3).map(|_| r.random_range(0..u64::MAX)).collect();
        assert_eq!(
            drawn,
            [
                12578764544318200737,
                7886285670807131020,
                5323617429756461743
            ]
        );
        assert_eq!(derive_seed(&[b"main", b"2026-01-05"]), 14072695264987897440);
        assert_ne!(
            derive_seed(&[b"ab", b"c"]),
            derive_seed(&[b"a", b"bc"]),
            "parts cannot run together"
        );
    }
}
