//! The password boundary: hashing, verification, and rehash policy checks.

use std::str::from_utf8;

use argon2::Argon2;
use argon2::PasswordHash;
use argon2::PasswordHasher;
use argon2::PasswordVerifier;
use argon2::password_hash::SaltString;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::value::Value;

const BCRYPT_SALT_LENGTH: usize = 16;

fn argon2_hash(
    context: &Context<'_, '_, '_>,
    algorithm: argon2::Algorithm,
    arguments: &Arguments<'_>,
) -> Value {
    let password = arguments.bytes(0);
    let memory_cost = arguments.int(1);
    let time_cost = arguments.int(2);
    let parallelism = arguments.int(3);
    let salt = arguments.bytes(4);

    let (Ok(memory_cost), Ok(time_cost), Ok(parallelism)) = (
        u32::try_from(memory_cost),
        u32::try_from(time_cost),
        u32::try_from(parallelism),
    ) else {
        return Value::null();
    };
    let Ok(parameters) = argon2::Params::new(memory_cost, time_cost, parallelism, None) else {
        return Value::null();
    };
    let Ok(salt) = SaltString::encode_b64(salt) else {
        return Value::null();
    };

    let hasher = Argon2::new(algorithm, argon2::Version::V0x13, parameters);
    let Ok(hash) = hasher.hash_password(password, &salt) else {
        return Value::null();
    };

    let rendered = hash.to_string();
    context.owned_string(rendered.into_bytes())
}

fn argon2_hash_parameters(hash: &str, variant: &str) -> Option<(u32, u32, u32)> {
    let parsed = PasswordHash::new(hash).ok()?;
    if parsed.algorithm.as_str() != variant {
        return None;
    }

    let parameters = argon2::Params::try_from(&parsed).ok()?;
    Some((
        parameters.m_cost(),
        parameters.t_cost(),
        parameters.p_cost(),
    ))
}

fn bcrypt_hash_cost(hash: &[u8]) -> Option<i64> {
    let hash = from_utf8(hash).ok()?;
    let mut fields = hash.strip_prefix('$')?.split('$');
    let version = fields.next()?;
    if !matches!(version, "2a" | "2b" | "2x" | "2y") {
        return None;
    }

    fields.next()?.parse::<u8>().ok().map(i64::from)
}

#[whim_function(
    "Whim\\_Private\\password_hash_bcrypt(#[SensitiveParameter] string $password, int $cost, string $salt): null|string"
)]
fn password_hash_bcrypt(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    let password = arguments.bytes(0);
    let cost = arguments.int(1);
    let salt = arguments.bytes(2);
    let Ok(cost) = u32::try_from(cost) else {
        return Value::null();
    };
    let Ok(salt) = <[u8; BCRYPT_SALT_LENGTH]>::try_from(salt) else {
        return Value::null();
    };

    let Ok(parts) = bcrypt::hash_with_salt(password, cost, salt) else {
        return Value::null();
    };

    let rendered = parts.format_for_version(bcrypt::Version::TwoB);
    context.owned_string(rendered.into_bytes())
}

#[whim_function(
    "Whim\\_Private\\password_hash_argon2i(#[SensitiveParameter] string $password, int $memoryCost, int $timeCost, int $parallelism, string $salt): null|string"
)]
fn password_hash_argon2i(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    argon2_hash(context, argon2::Algorithm::Argon2i, &arguments)
}

#[whim_function(
    "Whim\\_Private\\password_hash_argon2id(#[SensitiveParameter] string $password, int $memoryCost, int $timeCost, int $parallelism, string $salt): null|string"
)]
fn password_hash_argon2id(context: &Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    argon2_hash(context, argon2::Algorithm::Argon2id, &arguments)
}

#[whim_function(
    "Whim\\_Private\\password_verify(#[SensitiveParameter] string $password, #[SensitiveParameter] string $hash): bool"
)]
fn password_verify(arguments: Arguments<'_>) -> Value {
    let password = arguments.bytes(0);
    let hash = arguments.bytes(1);
    let Ok(hash) = from_utf8(hash) else {
        return Value::bool(false);
    };

    if hash.starts_with("$2") {
        let verified = bcrypt::verify(password, hash).unwrap_or(false);
        return Value::bool(verified);
    }

    let Ok(parsed) = PasswordHash::new(hash) else {
        return Value::bool(false);
    };

    let verified = Argon2::default().verify_password(password, &parsed).is_ok();
    Value::bool(verified)
}

#[whim_function("Whim\\_Private\\password_needs_rehash_bcrypt(string $hash, int $cost): bool")]
fn password_needs_rehash_bcrypt(arguments: Arguments<'_>) -> Value {
    let hash = arguments.bytes(0);
    let cost = arguments.int(1);
    Value::bool(bcrypt_hash_cost(hash) != Some(cost))
}

#[whim_function(
    "Whim\\_Private\\password_needs_rehash_argon2i(string $hash, int $memoryCost, int $timeCost, int $parallelism): bool"
)]
fn password_needs_rehash_argon2i(arguments: Arguments<'_>) -> Value {
    needs_rehash_argon2(arguments, "argon2i")
}

#[whim_function(
    "Whim\\_Private\\password_needs_rehash_argon2id(string $hash, int $memoryCost, int $timeCost, int $parallelism): bool"
)]
fn password_needs_rehash_argon2id(arguments: Arguments<'_>) -> Value {
    needs_rehash_argon2(arguments, "argon2id")
}

fn needs_rehash_argon2(arguments: Arguments<'_>, variant: &str) -> Value {
    let hash = arguments.bytes(0);
    let memory_cost = arguments.int(1);
    let time_cost = arguments.int(2);
    let parallelism = arguments.int(3);
    let embedded = from_utf8(hash)
        .ok()
        .and_then(|hash| argon2_hash_parameters(hash, variant));
    let expected = (
        u32::try_from(memory_cost).ok(),
        u32::try_from(time_cost).ok(),
        u32::try_from(parallelism).ok(),
    );
    let matches = embedded
        .is_some_and(|(memory, time, lanes)| (Some(memory), Some(time), Some(lanes)) == expected);
    Value::bool(!matches)
}
