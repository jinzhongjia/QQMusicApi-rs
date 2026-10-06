//! Custom Triple-DES variant used by QQ Music to encrypt QRC lyrics.
//!
//! The key schedule intentionally keeps the non-standard PC-2 offset quirk
//! of the original implementation (QQMusicDecoder / upstream `tripledes.py`),
//! therefore this is *not* interoperable with standard 3DES.

/// Cipher direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Encrypt.
    Encrypt,
    /// Decrypt.
    Decrypt,
}

#[rustfmt::skip]
const SBOX: [[u8; 64]; 8] = [
    [14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11, 9, 5, 3, 8,
     4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13],
    [15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 15, 12, 0, 1, 10, 6, 9, 11, 5,
     0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9],
    [10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14, 12, 11, 15, 1,
     13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12],
    [7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12, 1, 10, 14, 9,
     10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10, 10, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14],
    [2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10, 3, 9, 8, 6,
     4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3],
    [12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14, 0, 11, 3, 8,
     9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13],
    [4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12, 2, 15, 8, 6,
     1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12],
    [13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11, 0, 14, 9, 2,
     7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11],
];

/// Output bit sources (for bit 31 down to 0) of the P permutation in `f`.
const P_PERM: [u32; 32] = [
    16, 25, 12, 11, 3, 20, 4, 15, 31, 17, 9, 6, 27, 14, 1, 22, 30, 24, 8, 18, 0, 5, 29, 23, 13, 19,
    2, 26, 10, 21, 28, 7,
];

const KEY_RND_SHIFT: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
const KEY_PERM_C: [u32; 28] = [
    56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59,
    51, 43, 35,
];
const KEY_PERM_D: [u32; 28] = [
    62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 60, 52, 44, 36, 28, 20, 12, 4, 27,
    19, 11, 3,
];
const KEY_COMPRESSION: [u32; 48] = [
    13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9, 22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12, 1, 40, 51,
    30, 36, 46, 54, 29, 39, 50, 44, 32, 47, 43, 48, 38, 55, 33, 52, 45, 41, 49, 35, 28, 31,
];

/// 16 round sub keys of 6 bytes each.
pub type KeySchedule = [[u8; 6]; 16];

/// Three DES key schedules for the triple-DES pipeline.
pub type TripleKeySchedule = [KeySchedule; 3];

#[inline]
fn bit(value: u32, shift: u32) -> u32 {
    (value >> shift) & 1
}

#[inline]
fn sbox_bit(a: u8) -> usize {
    usize::from((a & 32) | ((a & 31) >> 1) | ((a & 1) << 4))
}

fn initial_permutation(input: &[u8; 8]) -> (u32, u32) {
    let v0 = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
    let v1 = u32::from_le_bytes([input[4], input[5], input[6], input[7]]);
    let permute = |groups: [u32; 4]| {
        let mut out = 0u32;
        let mut position = 32u32;
        for group in groups {
            for source in [v1, v0] {
                for k in 0..4 {
                    position -= 1;
                    out |= bit(source, group + 8 * k) << position;
                }
            }
        }
        out
    };
    (permute([6, 4, 2, 0]), permute([7, 5, 3, 1]))
}

fn inverse_permutation(s0: u32, s1: u32) -> [u8; 8] {
    const TARGET: [usize; 8] = [3, 2, 1, 0, 7, 6, 5, 4];
    let mut data = [0u8; 8];
    for (offset, &index) in (0u32..).zip(TARGET.iter()) {
        let mut byte = 0u32;
        let mut position = 8u32;
        for k in [3u32, 2, 1, 0] {
            for source in [s1, s0] {
                position -= 1;
                byte |= bit(source, 8 * k + offset) << position;
            }
        }
        data[index] = u8::try_from(byte).unwrap_or_default();
    }
    data
}

#[allow(clippy::cast_possible_truncation)]
fn f(state: u32, key: &[u8; 6]) -> u32 {
    let t1 = ((state & 1) << 31)
        | ((state & 0xF800_0000) >> 1)
        | ((state & 0x1F80_0000) >> 3)
        | ((state & 0x01F8_0000) >> 5)
        | ((state & 0x001F_8000) >> 7);
    let t2 = ((state & 0x0001_F800) << 15)
        | ((state & 0x0000_1F80) << 13)
        | ((state & 0x0000_01F8) << 11)
        | ((state & 0x0000_001F) << 9)
        | ((state & 0x8000_0000) >> 23);

    let k0 = ((t1 >> 24) as u8) ^ key[0];
    let k1 = ((t1 >> 16) as u8) ^ key[1];
    let k2 = ((t1 >> 8) as u8) ^ key[2];
    let k3 = ((t2 >> 24) as u8) ^ key[3];
    let k4 = ((t2 >> 16) as u8) ^ key[4];
    let k5 = ((t2 >> 8) as u8) ^ key[5];

    let s = |table: usize, index: u8| u32::from(SBOX[table][sbox_bit(index)]);
    let state = (s(0, k0 >> 2) << 28)
        | (s(1, ((k0 & 0x03) << 4) | (k1 >> 4)) << 24)
        | (s(2, ((k1 & 0x0F) << 2) | (k2 >> 6)) << 20)
        | (s(3, k2 & 0x3F) << 16)
        | (s(4, k3 >> 2) << 12)
        | (s(5, ((k3 & 0x03) << 4) | (k4 >> 4)) << 8)
        | (s(6, ((k4 & 0x0F) << 2) | (k5 >> 6)) << 4)
        | s(7, k5 & 0x3F);

    P_PERM
        .iter()
        .enumerate()
        .fold(0u32, |acc, (i, &src)| acc | (bit(state, src) << (31 - i as u32)))
}

fn crypt(input: &[u8; 8], key: &KeySchedule) -> [u8; 8] {
    let (mut s0, mut s1) = initial_permutation(input);
    for round in key.iter().take(15) {
        let previous = s1;
        s1 = f(s1, round) ^ s0;
        s0 = previous;
    }
    s0 ^= f(s1, &key[15]);
    inverse_permutation(s0, s1)
}

/// DES key schedule (with the custom PC-2 quirk).
///
/// # Panics
///
/// Panics if `key` is shorter than 8 bytes.
#[allow(clippy::cast_possible_truncation)]
pub fn key_schedule(key: &[u8], mode: Mode) -> KeySchedule {
    let mut schedule: KeySchedule = [[0; 6]; 16];
    let v0 = u32::from_le_bytes([key[0], key[1], key[2], key[3]]);
    let v1 = u32::from_le_bytes([key[4], key[5], key[6], key[7]]);
    let pick = |b: u32| {
        if b < 32 {
            bit(v0, 31 - b)
        } else {
            bit(v1, 63 - b)
        }
    };

    let mut c = KEY_PERM_C
        .iter()
        .enumerate()
        .fold(0u32, |acc, (i, &b)| acc | (pick(b) << (31 - i as u32)));
    let mut d = KEY_PERM_D
        .iter()
        .enumerate()
        .fold(0u32, |acc, (i, &b)| acc | (pick(b) << (31 - i as u32)));

    for (i, shift) in KEY_RND_SHIFT.iter().enumerate() {
        c = ((c << shift) | (c >> (28 - shift))) & 0xFFFF_FFF0;
        d = ((d << shift) | (d >> (28 - shift))) & 0xFFFF_FFF0;
        let target = match mode {
            Mode::Decrypt => 15 - i,
            Mode::Encrypt => i,
        };
        let round = &mut schedule[target];
        *round = [0; 6];
        for (j, &comp) in KEY_COMPRESSION.iter().enumerate() {
            let value = if j < 24 {
                bit(c, 31 - comp)
            } else {
                bit(d, 31 - (comp - 27))
            };
            round[j / 8] |= (value as u8) << (7 - (j % 8));
        }
    }
    schedule
}

/// Triple-DES key setup.
///
/// # Panics
///
/// Panics if `key` is shorter than 24 bytes.
pub fn tripledes_key_setup(key: &[u8], mode: Mode) -> TripleKeySchedule {
    match mode {
        Mode::Encrypt => [
            key_schedule(&key[0..], Mode::Encrypt),
            key_schedule(&key[8..], Mode::Decrypt),
            key_schedule(&key[16..], Mode::Encrypt),
        ],
        Mode::Decrypt => [
            key_schedule(&key[16..], Mode::Decrypt),
            key_schedule(&key[8..], Mode::Encrypt),
            key_schedule(&key[0..], Mode::Decrypt),
        ],
    }
}

/// Encrypt or decrypt a single 8 byte block.
pub fn tripledes_crypt(block: &[u8; 8], key: &TripleKeySchedule) -> [u8; 8] {
    key.iter().fold(*block, |data, schedule| crypt(&data, schedule))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8; 24] = b"!@#)(*$%123ZXC!@!@#)(NHL";

    #[test]
    fn roundtrip() {
        let enc = tripledes_key_setup(KEY, Mode::Encrypt);
        let dec = tripledes_key_setup(KEY, Mode::Decrypt);
        for block in [*b"abcdefgh", [0u8; 8], [0xFF; 8], *b"\x01\x02\x03\x04\x05\x06\x07\x08"] {
            let encrypted = tripledes_crypt(&block, &enc);
            assert_ne!(encrypted, block);
            assert_eq!(tripledes_crypt(&encrypted, &dec), block);
        }
    }

    #[test]
    fn matches_python_reference_vectors() {
        // (plain, encrypted, decrypted) generated with upstream tripledes.py
        let vectors = [
            ("6162636465666768", "f95817db1b0c5191", "1b9dc45b771ec88f"),
            ("0000000000000000", "a27b02aa779bf226", "d4f80aa34ca0b59c"),
            ("ffffffffffffffff", "4b789c44381642a3", "5b0da514d2289913"),
            ("0102030405060708", "9eaea5720d901148", "6dc4f6a6b5e7cb96"),
        ];
        let enc = tripledes_key_setup(KEY, Mode::Encrypt);
        let dec = tripledes_key_setup(KEY, Mode::Decrypt);
        for (plain, encrypted, decrypted) in vectors {
            let block: [u8; 8] = hex::decode(plain).unwrap().try_into().unwrap();
            assert_eq!(hex::encode(tripledes_crypt(&block, &enc)), encrypted);
            assert_eq!(hex::encode(tripledes_crypt(&block, &dec)), decrypted);
        }
    }
}
