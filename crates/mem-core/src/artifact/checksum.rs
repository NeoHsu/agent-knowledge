use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};

use crate::error;

const SHA256_PREFIX: &str = "sha256:";

pub(super) fn valid_sha256_checksum(value: &str) -> bool {
    let Some(hex) = value.strip_prefix(SHA256_PREFIX) else {
        return false;
    };
    hex.len() == 64 && hex.as_bytes().iter().all(u8::is_ascii_hexdigit)
}

pub(super) fn file_sha256(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(error::safety_violation(format!(
            "refusing to hash non-regular file: {}",
            path.display()
        )));
    }
    let mut file = fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut state = sha256_initial_state();
    let mut remaining = metadata.len();
    let mut block = [0_u8; 64];
    while remaining >= 64 {
        file.read_exact(&mut block)?;
        compress_sha256(&mut state, &block);
        remaining -= 64;
    }
    let mut tail = vec![0_u8; remaining as usize];
    file.read_exact(&mut tail)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        return Err(error::integrity(format!(
            "file changed while hashing: {}",
            path.display()
        )));
    }
    finalize_sha256(&mut state, &tail, metadata.len())?;
    Ok(format!("{SHA256_PREFIX}{}", format_sha256(&state)))
}

pub fn artifact_file_checksum(path: &Path) -> Result<String> {
    file_sha256(path)
}

pub fn artifact_file_is_executable(path: &Path) -> Result<bool> {
    is_executable(path)
}

#[cfg(unix)]
pub(super) fn is_executable(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    Ok(metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
pub(super) fn is_executable(path: &Path) -> Result<bool> {
    let metadata = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    Ok(metadata.is_file())
}

#[cfg(test)]
fn sha256_hex(input: &[u8]) -> String {
    let mut state = sha256_initial_state();
    let mut chunks = input.chunks_exact(64);
    for chunk in &mut chunks {
        let block: &[u8; 64] = chunk.try_into().expect("SHA-256 block length");
        compress_sha256(&mut state, block);
    }
    finalize_sha256(&mut state, chunks.remainder(), input.len() as u64)
        .expect("in-memory SHA-256 input length");
    format_sha256(&state)
}

fn sha256_initial_state() -> [u32; 8] {
    [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ]
}

fn finalize_sha256(state: &mut [u32; 8], tail: &[u8], total_bytes: u64) -> Result<()> {
    if tail.len() >= 64 {
        return Err(error::integrity("invalid SHA-256 tail length"));
    }
    let bit_len = total_bytes
        .checked_mul(8)
        .ok_or_else(|| error::usage("file is too large to hash with SHA-256"))?;
    let mut padding = Vec::with_capacity(128);
    padding.extend_from_slice(tail);
    padding.push(0x80);
    while padding.len() % 64 != 56 {
        padding.push(0);
    }
    padding.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padding.chunks_exact(64) {
        let block: &[u8; 64] = chunk.try_into().expect("SHA-256 padding block length");
        compress_sha256(state, block);
    }
    Ok(())
}

fn compress_sha256(state: &mut [u32; 8], chunk: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (index, word) in chunk.chunks_exact(4).take(16).enumerate() {
        w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
    }
    for index in 16..64 {
        let s0 =
            w[index - 15].rotate_right(7) ^ w[index - 15].rotate_right(18) ^ (w[index - 15] >> 3);
        let s1 =
            w[index - 2].rotate_right(17) ^ w[index - 2].rotate_right(19) ^ (w[index - 2] >> 10);
        w[index] = w[index - 16]
            .wrapping_add(s0)
            .wrapping_add(w[index - 7])
            .wrapping_add(s1);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for index in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[index])
            .wrapping_add(w[index]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

fn format_sha256(state: &[u32; 8]) -> String {
    state
        .iter()
        .map(|word| format!("{word:08x}"))
        .collect::<String>()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
}
