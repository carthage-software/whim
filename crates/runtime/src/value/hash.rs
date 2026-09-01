//! Per-engine keyed hashing for runtime values.

use std::collections::hash_map::DefaultHasher;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::hash::Hasher;

const INT_SEED_DOMAIN: u64 = 0x7df2_720f_8d2d_878d;
const BOOL_SEED_DOMAIN: u64 = 0xf8d2_84dd_44bd_aebd;
const SHORT_STRING_SEED_DOMAIN: u64 = 0x65a1_14da_5446_886d;
const STRING_HASH_DOMAIN: u8 = 2;

pub(crate) struct HashState {
    random: RandomState,
    int_seed: u64,
    bool_seed: u64,
    short_string_seed: u64,
}

#[expect(
    clippy::inline_always,
    reason = "scalar hashing runs inside dictionary lookup hot paths"
)]
impl HashState {
    #[must_use]
    pub(crate) fn new() -> Self {
        let random = RandomState::new();
        Self {
            int_seed: derive_seed(&random, INT_SEED_DOMAIN),
            bool_seed: derive_seed(&random, BOOL_SEED_DOMAIN),
            short_string_seed: derive_seed(&random, SHORT_STRING_SEED_DOMAIN),
            random,
        }
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn hash_int(&self, value: i64) -> u64 {
        permute(value.cast_unsigned(), self.int_seed)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn hash_bool(&self, value: bool) -> u64 {
        permute(value as u64, self.bool_seed)
    }

    #[must_use]
    #[inline(always)]
    pub(crate) const fn hash_short_string(&self, packed: u64) -> u64 {
        permute(packed, self.short_string_seed)
    }

    pub(crate) fn string_hasher(&self, len: usize) -> impl Hasher {
        let mut hasher = self.random.build_hasher();
        hasher.write_u8(STRING_HASH_DOMAIN);
        hasher.write_usize(len);
        hasher
    }

    pub(crate) fn structural_hasher(&self) -> DefaultHasher {
        self.random.build_hasher()
    }
}

fn derive_seed(state: &RandomState, domain: u64) -> u64 {
    let mut hasher = state.build_hasher();
    hasher.write_u64(domain);
    hasher.finish()
}

#[expect(
    clippy::inline_always,
    reason = "the keyed permutation is the scalar hashing hot path"
)]
#[inline(always)]
const fn permute(mut value: u64, seed: u64) -> u64 {
    value ^= seed;
    value ^= value >> 32;
    value = value.wrapping_mul(0xd6e8_feb8_6659_fd93);
    value ^ (value >> 32)
}
