//! Hash primitives NTLMv2 needs, and nothing else.
//!
//! NTLMv2 is built from MD4 (the password hash), HMAC-MD5 (the response key)
//! and MD5 (the message digest behind it). `ring`, the crypto provider this
//! crate already uses, deliberately leaves out MD4 and MD5, and no digest crate
//! with them is available offline, so both live here. They are used only to
//! compute an authentication response for a server that asks for it; nothing
//! secret is stored in terms of them.
//!
//! Verified against the RFC 1320 / RFC 1321 / RFC 2202 vectors below.

/// MD4 (RFC 1320). Only NTLMv2 uses it, as `MD4(UTF-16LE(password))`.
pub(crate) fn md4(input: &[u8]) -> [u8; 16] {
    const S1: [u32; 4] = [3, 7, 11, 19];
    const S2: [u32; 4] = [3, 5, 9, 13];
    const S3: [u32; 4] = [3, 9, 11, 15];
    const K2: u32 = 0x5a82_7999;
    const K3: u32 = 0x6ed9_eba1;

    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in padded(input).chunks_exact(64) {
        let mut x = [0u32; 16];
        for (word, bytes) in chunk.chunks_exact(4).enumerate() {
            x[word] = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for index in 0..16 {
            let f = (b & c) | (!b & d);
            let t = a
                .wrapping_add(f)
                .wrapping_add(x[index])
                .rotate_left(S1[index % 4]);
            a = d;
            d = c;
            c = b;
            b = t;
        }
        for index in 0..16 {
            let g = (b & c) | (b & d) | (c & d);
            let t = a
                .wrapping_add(g)
                .wrapping_add(x[(index % 4) * 4 + index / 4])
                .wrapping_add(K2)
                .rotate_left(S2[index % 4]);
            a = d;
            d = c;
            c = b;
            b = t;
        }
        for index in 0..16 {
            const ORDER: [usize; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
            let h = b ^ c ^ d;
            let t = a
                .wrapping_add(h)
                .wrapping_add(x[ORDER[index]])
                .wrapping_add(K3)
                .rotate_left(S3[index % 4]);
            a = d;
            d = c;
            c = b;
            b = t;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    digest_bytes(state)
}

/// MD5 (RFC 1321), used by [`hmac_md5`].
pub(crate) fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76a_a478,
        0xe8c7_b756,
        0x2420_70db,
        0xc1bd_ceee,
        0xf57c_0faf,
        0x4787_c62a,
        0xa830_4613,
        0xfd46_9501,
        0x6980_98d8,
        0x8b44_f7af,
        0xffff_5bb1,
        0x895c_d7be,
        0x6b90_1122,
        0xfd98_7193,
        0xa679_438e,
        0x49b4_0821,
        0xf61e_2562,
        0xc040_b340,
        0x265e_5a51,
        0xe9b6_c7aa,
        0xd62f_105d,
        0x0244_1453,
        0xd8a1_e681,
        0xe7d3_fbc8,
        0x21e1_cde6,
        0xc337_07d6,
        0xf4d5_0d87,
        0x455a_14ed,
        0xa9e3_e905,
        0xfcef_a3f8,
        0x676f_02d9,
        0x8d2a_4c8a,
        0xfffa_3942,
        0x8771_f681,
        0x6d9d_6122,
        0xfde5_380c,
        0xa4be_ea44,
        0x4bde_cfa9,
        0xf6bb_4b60,
        0xbebf_bc70,
        0x289b_7ec6,
        0xeaa1_27fa,
        0xd4ef_3085,
        0x0488_1d05,
        0xd9d4_d039,
        0xe6db_99e5,
        0x1fa2_7cf8,
        0xc4ac_5665,
        0xf429_2244,
        0x432a_ff97,
        0xab94_23a7,
        0xfc93_a039,
        0x655b_59c3,
        0x8f0c_cc92,
        0xffef_f47d,
        0x8584_5dd1,
        0x6fa8_7e4f,
        0xfe2c_e6e0,
        0xa301_4314,
        0x4e08_11a1,
        0xf753_7e82,
        0xbd3a_f235,
        0x2ad7_d2bb,
        0xeb86_d391,
    ];

    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in padded(input).chunks_exact(64) {
        let mut m = [0u32; 16];
        for (word, bytes) in chunk.chunks_exact(4).enumerate() {
            m[word] = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for step in 0..64 {
            let (f, index) = match step / 16 {
                0 => ((b & c) | (!b & d), step),
                1 => ((d & b) | (!d & c), (5 * step + 1) % 16),
                2 => (b ^ c ^ d, (3 * step + 5) % 16),
                _ => (c ^ (b | !d), (7 * step) % 16),
            };
            let t = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[step])
                    .wrapping_add(m[index])
                    .rotate_left(S[step]),
            );
            a = d;
            d = c;
            c = b;
            b = t;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    digest_bytes(state)
}

/// HMAC-MD5 (RFC 2104), the keyed hash NTLMv2 is defined in terms of.
pub(crate) fn hmac_md5(key: &[u8], message: &[u8]) -> [u8; 16] {
    let mut block = [0u8; 64];
    if key.len() > block.len() {
        block[..16].copy_from_slice(&md5(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend(block.iter().map(|byte| byte ^ 0x36));
    inner.extend_from_slice(message);
    let mut outer = Vec::with_capacity(80);
    outer.extend(block.iter().map(|byte| byte ^ 0x5c));
    outer.extend_from_slice(&md5(&inner));
    md5(&outer)
}

/// UTF-16LE, the encoding every NTLM string field uses.
pub(crate) fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<u8>>()
}

/// MD4/MD5 padding: `0x80`, zeroes to 56 mod 64, then the bit length little-endian.
fn padded(input: &[u8]) -> Vec<u8> {
    let mut message = input.to_vec();
    let bits = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_le_bytes());
    message
}

fn digest_bytes(state: [u32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (index, word) in state.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn md4_matches_the_rfc_1320_vectors() {
        for (input, expected) in [
            ("", "31d6cfe0d16ae931b73c59d7e0c089c0"),
            ("a", "bde52cb31de33e46245e05fbdbd6fb24"),
            ("abc", "a448017aaf21d8525fc10ae87aa6729d"),
            ("message digest", "d9130a8164549fe818874806e1c7014b"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "d79e1c308aa5bbcdeea8ed63df412da9",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "e33b4ddc9c38f2199c3e7b164fcc0536",
            ),
        ] {
            assert_eq!(hex(&md4(input.as_bytes())), expected, "md4({input:?})");
        }
    }

    #[test]
    fn md5_matches_the_rfc_1321_vectors() {
        for (input, expected) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            assert_eq!(hex(&md5(input.as_bytes())), expected, "md5({input:?})");
        }
    }

    /// RFC 2202 §2, test cases 1–3 and 6.
    #[test]
    fn hmac_md5_matches_the_rfc_2202_vectors() {
        assert_eq!(
            hex(&hmac_md5(&[0x0b; 16], b"Hi There")),
            "9294727a3638bb1c13f48ef8158bfc9d"
        );
        assert_eq!(
            hex(&hmac_md5(b"Jefe", b"what do ya want for nothing?")),
            "750c783e6ab0b503eaa86e310a5db738"
        );
        assert_eq!(
            hex(&hmac_md5(&[0xaa; 16], &[0xdd; 50])),
            "56be34521d144c88dbb8c733f0e8b3f6"
        );
        assert_eq!(
            hex(&hmac_md5(
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "6b1ab7fe4bd7bf8f0b62e6ce61b9d0cd"
        );
    }

    #[test]
    fn utf16le_encodes_every_basic_multilingual_character() {
        assert_eq!(utf16le("abc"), b"a\0b\0c\0");
        assert_eq!(utf16le("密码"), vec![0xc6, 0x5b, 0x01, 0x78]);
    }
}
