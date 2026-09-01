# Hashes, Passwords, and Random Data

## Hashes

`Hash\Algorithm` names SHA-2, SHA-3, Keccak-256, SHA-1, MD5, RIPEMD-160,
BLAKE3, BLAKE2b-256, CRC, Adler, and xxHash forms. Each case reports its digest
length and whether it is cryptographic.

`Hash\digest($algorithm, $bytes, $hex)` computes one digest. `Hasher` accepts
chunks through `update` and returns bytes from one final `finish`.

`Hash\hmac` computes a keyed MAC for a supported digest. `HmacHasher` is its
streaming form. `Hash\equals` compares secret byte strings in fixed time for
their length.

`Hash\pbkdf2_sha256($password, $salt, $iterations)` derives 32 bytes with
PBKDF2-HMAC-SHA-256. The iteration count must be between 1 and 1,000,000.

Do not use CRC, Adler, xxHash, MD5, or SHA-1 for a new security check. Their
presence supports checksums and old formats.

## Password hashes

`Password\hash($password, $algorithm)` creates a fresh salt and returns a
self-describing hash. Algorithms are `Bcrypt`, `Argon2i`, and `Argon2id`, with
checked cost settings.

`verify` checks a password and returns false for an unknown hash.
`needs_rehash` reports whether a stored hash uses a different algorithm or cost.

Bcrypt accepts at most 72 password bytes; the function rejects longer input
instead of truncating it. Traces hide passwords and stored hashes.

## Secure random data

`SecureRandom\bytes($length)` reads operating-system random bytes.
`string($length, $alphabet)` selects unbiased characters from an alphabet.
`int($min, $max)` selects an inclusive unbiased integer. `float()` returns a
value from zero inclusive to one exclusive.

The default alphabet for a random string is safe for common text uses. Supply
an explicit alphabet when a protocol has a fixed one.

Failure to obtain enough operating-system randomness throws
`InsufficientEntropyException`.

## Pseudo-random sequences

`PseudoRandom\int` and `float` use one process sequence seeded from secure
random bytes. They are suitable for tests, sampling, games, and shuffling, not
for keys, tokens, salts, or passwords.

`RandomSequence\MersenneTwisterSequence` accepts an explicit 32-bit seed and is
repeatable. `SecureSequence` reads fresh secure data. Both implement `Sequence`
with `next`, `nextFloat`, and inclusive `nextIn`.

Use an explicit sequence object when a test needs the same output on every run.

## UUID

`UUID\UUID::v4()` creates a random UUID. `v7()` creates a time-ordered UUID.
`parse` accepts canonical lowercase text or returns `null`; `from` throws.

`fromBytes` requires exactly 16 bytes. `toBytes`, `toString`, `version`, and
`equals` expose the value without changing it. Parsed UUIDs may have no known
version.
