//! The hashing boundary: algorithm numbers, one-shot digests, and streaming state.

use std::cell::RefCell;

use blake2::Digest as _;
use blake2::digest::consts::U32;
use hmac::Hmac;
use hmac::KeyInit;
use hmac::Mac;
use hmac::SimpleHmac;
use hmac::digest::Digest as HmacDigest;
use hmac::digest::block_api::EagerHash;
use hmac::digest::common::BlockSizeUser;
use xxhash_rust::xxh3::Xxh3;
use xxhash_rust::xxh64::Xxh64;

use whim_macros::whim_class;
use whim_macros::whim_constant;
use whim_macros::whim_function;
use whim_macros::whim_methods;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::convert::state_ref;
use crate::builtin::throw::Throw;
use crate::unwrap_option_invariant;
use crate::unwrap_result_invariant;
use crate::value::Value;

type Blake2b256 = blake2::Blake2b<U32>;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA2_224", "int")]
pub(crate) const HASH_ALGORITHM_SHA2_224: i64 = 1;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA2_256", "int")]
pub(crate) const HASH_ALGORITHM_SHA2_256: i64 = 2;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA2_384", "int")]
pub(crate) const HASH_ALGORITHM_SHA2_384: i64 = 3;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA2_512", "int")]
pub(crate) const HASH_ALGORITHM_SHA2_512: i64 = 4;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA2_512_256", "int")]
pub(crate) const HASH_ALGORITHM_SHA2_512_256: i64 = 5;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA3_224", "int")]
pub(crate) const HASH_ALGORITHM_SHA3_224: i64 = 6;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA3_256", "int")]
pub(crate) const HASH_ALGORITHM_SHA3_256: i64 = 7;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA3_384", "int")]
pub(crate) const HASH_ALGORITHM_SHA3_384: i64 = 8;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA3_512", "int")]
pub(crate) const HASH_ALGORITHM_SHA3_512: i64 = 9;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_KECCAK_256", "int")]
pub(crate) const HASH_ALGORITHM_KECCAK_256: i64 = 10;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_SHA1", "int")]
pub(crate) const HASH_ALGORITHM_SHA1: i64 = 11;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_MD5", "int")]
pub(crate) const HASH_ALGORITHM_MD5: i64 = 12;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_RIPEMD160", "int")]
pub(crate) const HASH_ALGORITHM_RIPEMD160: i64 = 13;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_BLAKE3", "int")]
pub(crate) const HASH_ALGORITHM_BLAKE3: i64 = 14;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_BLAKE2B_256", "int")]
pub(crate) const HASH_ALGORITHM_BLAKE2B_256: i64 = 15;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_CRC32", "int")]
pub(crate) const HASH_ALGORITHM_CRC32: i64 = 16;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_CRC32C", "int")]
pub(crate) const HASH_ALGORITHM_CRC32C: i64 = 17;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_ADLER32", "int")]
pub(crate) const HASH_ALGORITHM_ADLER32: i64 = 18;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_XXHASH64", "int")]
pub(crate) const HASH_ALGORITHM_XXHASH64: i64 = 19;

#[whim_constant("Whim\\_Private\\HASH_ALGORITHM_XXHASH3", "int")]
pub(crate) const HASH_ALGORITHM_XXHASH3: i64 = 20;

enum DigestState {
    Sha2_224(sha2::Sha224),
    Sha2_256(sha2::Sha256),
    Sha2_384(sha2::Sha384),
    Sha2_512(sha2::Sha512),
    Sha2_512_256(sha2::Sha512_256),
    Sha3_224(sha3::Sha3_224),
    Sha3_256(sha3::Sha3_256),
    Sha3_384(sha3::Sha3_384),
    Sha3_512(sha3::Sha3_512),
    Keccak256(sha3::Keccak256),
    Sha1(sha1::Sha1),
    Md5(md5::Md5),
    Ripemd160(ripemd::Ripemd160),
    Blake3(Box<blake3::Hasher>),
    Blake2b256(Blake2b256),
    Crc32(crc32fast::Hasher),
    Crc32c(u32),
    Adler32(simd_adler32::Adler32),
    Xxhash64(Xxh64),
    Xxhash3(Box<Xxh3>),
}

impl DigestState {
    fn begin(algorithm: i64) -> Option<Self> {
        Some(match algorithm {
            HASH_ALGORITHM_SHA2_224 => Self::Sha2_224(sha2::Sha224::new()),
            HASH_ALGORITHM_SHA2_256 => Self::Sha2_256(sha2::Sha256::new()),
            HASH_ALGORITHM_SHA2_384 => Self::Sha2_384(sha2::Sha384::new()),
            HASH_ALGORITHM_SHA2_512 => Self::Sha2_512(sha2::Sha512::new()),
            HASH_ALGORITHM_SHA2_512_256 => Self::Sha2_512_256(sha2::Sha512_256::new()),
            HASH_ALGORITHM_SHA3_224 => Self::Sha3_224(sha3::Sha3_224::new()),
            HASH_ALGORITHM_SHA3_256 => Self::Sha3_256(sha3::Sha3_256::new()),
            HASH_ALGORITHM_SHA3_384 => Self::Sha3_384(sha3::Sha3_384::new()),
            HASH_ALGORITHM_SHA3_512 => Self::Sha3_512(sha3::Sha3_512::new()),
            HASH_ALGORITHM_KECCAK_256 => Self::Keccak256(sha3::Keccak256::new()),
            HASH_ALGORITHM_SHA1 => Self::Sha1(sha1::Sha1::new()),
            HASH_ALGORITHM_MD5 => Self::Md5(md5::Md5::new()),
            HASH_ALGORITHM_RIPEMD160 => Self::Ripemd160(ripemd::Ripemd160::new()),
            HASH_ALGORITHM_BLAKE3 => Self::Blake3(Box::default()),
            HASH_ALGORITHM_BLAKE2B_256 => Self::Blake2b256(Blake2b256::new()),
            HASH_ALGORITHM_CRC32 => Self::Crc32(crc32fast::Hasher::new()),
            HASH_ALGORITHM_CRC32C => Self::Crc32c(0),
            HASH_ALGORITHM_ADLER32 => Self::Adler32(simd_adler32::Adler32::new()),
            HASH_ALGORITHM_XXHASH64 => Self::Xxhash64(Xxh64::new(0)),
            HASH_ALGORITHM_XXHASH3 => Self::Xxhash3(Box::default()),
            _ => return None,
        })
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha2_224(state) => state.update(bytes),
            Self::Sha2_256(state) => state.update(bytes),
            Self::Sha2_384(state) => state.update(bytes),
            Self::Sha2_512(state) => state.update(bytes),
            Self::Sha2_512_256(state) => state.update(bytes),
            Self::Sha3_224(state) => state.update(bytes),
            Self::Sha3_256(state) => state.update(bytes),
            Self::Sha3_384(state) => state.update(bytes),
            Self::Sha3_512(state) => state.update(bytes),
            Self::Keccak256(state) => state.update(bytes),
            Self::Sha1(state) => state.update(bytes),
            Self::Md5(state) => state.update(bytes),
            Self::Ripemd160(state) => state.update(bytes),
            Self::Blake3(state) => {
                state.update(bytes);
            }
            Self::Blake2b256(state) => state.update(bytes),
            Self::Crc32(state) => state.update(bytes),
            Self::Crc32c(state) => *state = crc32c::crc32c_append(*state, bytes),
            Self::Adler32(state) => simd_adler32::Adler32::write(state, bytes),
            Self::Xxhash64(state) => state.update(bytes),
            Self::Xxhash3(state) => state.update(bytes),
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Self::Sha2_224(state) => state.finalize().to_vec(),
            Self::Sha2_256(state) => state.finalize().to_vec(),
            Self::Sha2_384(state) => state.finalize().to_vec(),
            Self::Sha2_512(state) => state.finalize().to_vec(),
            Self::Sha2_512_256(state) => state.finalize().to_vec(),
            Self::Sha3_224(state) => state.finalize().to_vec(),
            Self::Sha3_256(state) => state.finalize().to_vec(),
            Self::Sha3_384(state) => state.finalize().to_vec(),
            Self::Sha3_512(state) => state.finalize().to_vec(),
            Self::Keccak256(state) => state.finalize().to_vec(),
            Self::Sha1(state) => state.finalize().to_vec(),
            Self::Md5(state) => state.finalize().to_vec(),
            Self::Ripemd160(state) => state.finalize().to_vec(),
            Self::Blake3(state) => state.finalize().as_bytes().to_vec(),
            Self::Blake2b256(state) => state.finalize().to_vec(),
            Self::Crc32(state) => state.finalize().to_be_bytes().to_vec(),
            Self::Crc32c(state) => state.to_be_bytes().to_vec(),
            Self::Adler32(state) => state.finish().to_be_bytes().to_vec(),
            Self::Xxhash64(state) => state.digest().to_be_bytes().to_vec(),
            Self::Xxhash3(state) => state.digest().to_be_bytes().to_vec(),
        }
    }
}

trait HmacComputation {
    fn update(&mut self, bytes: &[u8]);

    fn finish(self: Box<Self>) -> Vec<u8>;
}

impl<T: Mac + 'static> HmacComputation for T {
    fn update(&mut self, bytes: &[u8]) {
        Mac::update(self, bytes);
    }

    fn finish(self: Box<Self>) -> Vec<u8> {
        Mac::finalize(*self).into_bytes().to_vec()
    }
}

struct FallbackHmac {
    algorithm: i64,
    outer_key: Vec<u8>,
    inner: DigestState,
}

impl FallbackHmac {
    fn begin(algorithm: i64, key: &[u8], block_length: usize) -> Self {
        let normalized = if key.len() > block_length {
            // SAFETY: the surrounding invariant proves this option contains a value.
            let mut state = unsafe {
                unwrap_option_invariant(
                    DigestState::begin(algorithm),
                    "a fallback HMAC algorithm is a digest",
                )
            };
            state.update(key);
            state.finish()
        } else {
            key.to_vec()
        };
        let mut inner_key = vec![0x36; block_length];
        let mut outer_key = vec![0x5c; block_length];
        for (index, byte) in normalized.into_iter().enumerate() {
            inner_key[index] ^= byte;
            outer_key[index] ^= byte;
        }

        // SAFETY: the surrounding invariant proves this option contains a value.
        let mut inner = unsafe {
            unwrap_option_invariant(
                DigestState::begin(algorithm),
                "a fallback HMAC algorithm is a digest",
            )
        };
        inner.update(&inner_key);

        Self {
            algorithm,
            outer_key,
            inner,
        }
    }
}

impl HmacComputation for FallbackHmac {
    fn update(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    fn finish(self: Box<Self>) -> Vec<u8> {
        let inner = self.inner.finish();
        // SAFETY: the surrounding invariant proves this option contains a value.
        let mut outer = unsafe {
            unwrap_option_invariant(
                DigestState::begin(self.algorithm),
                "a fallback HMAC algorithm is a digest",
            )
        };
        outer.update(&self.outer_key);
        outer.update(&inner);
        outer.finish()
    }
}

fn hmac_computation<T>(key: &[u8]) -> Box<dyn HmacComputation>
where
    Hmac<T>: KeyInit + Mac + 'static,
    T: EagerHash,
{
    // SAFETY: the surrounding invariant proves this result is successful.
    let state = unsafe {
        unwrap_result_invariant(
            <Hmac<T> as KeyInit>::new_from_slice(key),
            "the HMAC implementation accepts keys of every length",
        )
    };
    Box::new(state)
}

fn simple_hmac_computation<T>(key: &[u8]) -> Box<dyn HmacComputation>
where
    SimpleHmac<T>: KeyInit + Mac + 'static,
    T: HmacDigest + BlockSizeUser,
{
    // SAFETY: the surrounding invariant proves this result is successful.
    let state = unsafe {
        unwrap_result_invariant(
            <SimpleHmac<T> as KeyInit>::new_from_slice(key),
            "the HMAC implementation accepts keys of every length",
        )
    };
    Box::new(state)
}

fn begin_hmac(algorithm: i64, key: &[u8]) -> Option<Box<dyn HmacComputation>> {
    Some(match algorithm {
        HASH_ALGORITHM_SHA2_224 => hmac_computation::<sha2::Sha224>(key),
        HASH_ALGORITHM_SHA2_256 => hmac_computation::<sha2::Sha256>(key),
        HASH_ALGORITHM_SHA2_384 => hmac_computation::<sha2::Sha384>(key),
        HASH_ALGORITHM_SHA2_512 => hmac_computation::<sha2::Sha512>(key),
        HASH_ALGORITHM_SHA2_512_256 => hmac_computation::<sha2::Sha512_256>(key),
        HASH_ALGORITHM_SHA3_224 => simple_hmac_computation::<sha3::Sha3_224>(key),
        HASH_ALGORITHM_SHA3_256 => simple_hmac_computation::<sha3::Sha3_256>(key),
        HASH_ALGORITHM_SHA3_384 => simple_hmac_computation::<sha3::Sha3_384>(key),
        HASH_ALGORITHM_SHA3_512 => simple_hmac_computation::<sha3::Sha3_512>(key),
        HASH_ALGORITHM_KECCAK_256 => simple_hmac_computation::<sha3::Keccak256>(key),
        HASH_ALGORITHM_SHA1 => hmac_computation::<sha1::Sha1>(key),
        HASH_ALGORITHM_MD5 => hmac_computation::<md5::Md5>(key),
        HASH_ALGORITHM_RIPEMD160 => hmac_computation::<ripemd::Ripemd160>(key),
        HASH_ALGORITHM_BLAKE3 => Box::new(FallbackHmac::begin(algorithm, key, 64)),
        HASH_ALGORITHM_BLAKE2B_256 => Box::new(FallbackHmac::begin(algorithm, key, 128)),
        _ => return None,
    })
}

#[expect(
    clippy::large_enum_variant,
    reason = "boxing digest state would add an allocation to every incremental digest"
)]
enum HashComputation {
    Digest(DigestState),
    Hmac(Box<dyn HmacComputation>),
}

impl HashComputation {
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Digest(state) => state.update(bytes),
            Self::Hmac(state) => state.update(bytes),
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Self::Digest(state) => state.finish(),
            Self::Hmac(state) => state.finish(),
        }
    }
}

#[whim_class("Whim\\_Private\\HashState", final)]
#[derive(Default)]
pub(crate) struct HashState {
    state: RefCell<Option<HashComputation>>,
}

default_built_in_state!(HashState);

#[whim_methods]
impl HashState {
    #[whim_method("__construct(): void", visibility = "private")]
    const fn construct() {}
}

fn unknown_algorithm(context: &mut Context<'_, '_, '_>) -> Throw {
    let class = context.vm.intern(b"Whim\\Unwind\\TypeError");
    context.vm.throw(
        class,
        "argument 1 ($algorithm) must name a hash algorithm",
        0,
    )
}

fn unknown_hmac_algorithm(context: &mut Context<'_, '_, '_>) -> Throw {
    let class = context.vm.intern(b"Whim\\Unwind\\TypeError");
    context.vm.throw(
        class,
        "argument 1 ($algorithm) must name a cryptographic hash algorithm",
        0,
    )
}

#[whim_function(
    "Whim\\_Private\\hash_digest(int $algorithm, #[SensitiveParameter] string $bytes): string"
)]
fn hash_digest(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let algorithm = arguments.int(0);
    let bytes = arguments.bytes(1);
    let Some(mut state) = DigestState::begin(algorithm) else {
        return Err(unknown_algorithm(context));
    };

    state.update(bytes);
    let digest = state.finish();
    Ok(context.string(&digest))
}

#[whim_function(
    "Whim\\_Private\\hash_hmac(int $algorithm, #[SensitiveParameter] string $key, #[SensitiveParameter] string $message): string"
)]
fn hash_hmac(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Result<Value, Throw> {
    let algorithm = arguments.int(0);
    let key = arguments.bytes(1);
    let message = arguments.bytes(2);
    let Some(mut state) = begin_hmac(algorithm, key) else {
        return Err(unknown_hmac_algorithm(context));
    };

    state.update(message);
    let digest = state.finish();
    Ok(context.string(&digest))
}

#[whim_function(
    "Whim\\_Private\\pbkdf2_sha256(#[SensitiveParameter] string $password, string $salt, 1..=1000000 $iterations): string[32]"
)]
fn pbkdf2_sha256(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let password = arguments.bytes(0);
    let salt = arguments.bytes(1);
    // SAFETY: the surrounding invariant proves this result is successful.
    let iterations = unsafe {
        unwrap_result_invariant(
            u32::try_from(arguments.int(2)),
            "a validated PBKDF2 iteration count fits in u32",
        )
    };
    let mut output = [0; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, iterations, &mut output);
    context.string(&output)
}

#[whim_function("Whim\\_Private\\create_hash_state(int $algorithm): Whim\\_Private\\HashState")]
fn create_hash_state(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let algorithm = arguments.int(0);
    let Some(state) = DigestState::begin(algorithm) else {
        return Err(unknown_algorithm(context));
    };

    let object = context.new_built_in_instance("Whim\\_Private\\HashState")?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<HashState>(&object),
            "a new hash state has its built-in state",
        )
    };
    built_in.state.replace(Some(HashComputation::Digest(state)));
    Ok(object)
}

#[whim_function(
    "Whim\\_Private\\create_hmac_state(int $algorithm, #[SensitiveParameter] string $key): Whim\\_Private\\HashState"
)]
fn create_hmac_state(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let algorithm = arguments.int(0);
    let key = arguments.bytes(1);
    let Some(state) = begin_hmac(algorithm, key) else {
        return Err(unknown_hmac_algorithm(context));
    };

    let object = context.new_built_in_instance("Whim\\_Private\\HashState")?;
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<HashState>(&object),
            "a new hash state has its built-in state",
        )
    };
    built_in.state.replace(Some(HashComputation::Hmac(state)));
    Ok(object)
}

#[whim_function(
    "Whim\\_Private\\update_hash_state(Whim\\_Private\\HashState $state, #[SensitiveParameter] string $bytes): bool"
)]
fn update_hash_state(arguments: Arguments<'_>) -> Value {
    let object = arguments.local(0);
    let bytes = arguments.bytes(1);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<HashState>(&object),
            "a validated hash state has built-in state",
        )
    };
    let mut state = built_in.state.borrow_mut();
    let Some(state) = state.as_mut() else {
        return Value::bool(false);
    };

    state.update(bytes);
    Value::bool(true)
}

#[whim_function("Whim\\_Private\\finish_hash_state(Whim\\_Private\\HashState $state): null|string")]
fn finish_hash_state(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let object = arguments.local(0);
    // SAFETY: the surrounding invariant proves this option contains a value.
    let built_in = unsafe {
        unwrap_option_invariant(
            state_ref::<HashState>(&object),
            "a validated hash state has built-in state",
        )
    };
    let Some(state) = built_in.state.borrow_mut().take() else {
        return Value::null();
    };

    let digest = state.finish();
    context.string(&digest)
}

#[whim_function(
    "Whim\\_Private\\constant_time_string_equals(#[SensitiveParameter] string $known, #[SensitiveParameter] string $given): bool"
)]
fn constant_time_string_equals(arguments: Arguments<'_>) -> Value {
    let known = arguments.bytes(0);
    let given = arguments.bytes(1);
    Value::bool(constant_time_eq::constant_time_eq(known, given))
}
