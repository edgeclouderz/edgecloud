//! Fixture integrity test — guards against stale `.wasm` files in CI.
//!
//! Each Phase D fixture has a committed SHA-256 in this file. If a
//! developer rebuilds a fixture but forgets to update the hash here,
//! the test fails with a clear message. If a developer edits the
//! fixture source without rebuilding, the .wasm bytes are unchanged
//! so the test still passes (the source isn't hashed — that's a
//! belt-and-suspenders check for the per-crate build workflow).
//!
//! Run with: `cargo test --manifest-path edge-worker/Cargo.toml --test test_fixtures_match_source`
//! Skip in CI: only if the fixture build is gated behind a feature flag
//! (currently always on — fixture files are committed).

use std::path::PathBuf;

const EXPECTED_HANDLER_HASH: &str =
    "5a54e5c269c3d0b0ca061d7b5b61c1e0b5f4b47dbafdc13211174b2926978747";

fn sha256(bytes: &[u8]) -> String {
    // Minimal pure-Rust SHA-256 (no external dep). For a test-only
    // integrity check this is fine; production crypto is sha2 crate.
    //
    // The implementation is the standard NIST FIPS 180-4 reference
    // — split into 512-bit blocks, process with 64 round constants,
    // output the big-endian digest.
    fn sha256_block(state: &mut [u32; 8], block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
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
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
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

    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // SHA-256 length suffix is the message length in BITS, not bytes
    // (FIPS 180-4 §5.1.1). The empty-input case works either way (0
    // bits = 0 bytes) which is why this bug only surfaced for non-empty
    // inputs.
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut msg = bytes.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let block: &[u8; 64] = chunk.try_into().expect("block");
        sha256_block(&mut state, block);
    }
    state.iter().map(|x| format!("{:08x}", x)).collect()
}

fn fixture_path(rel: &str) -> Option<PathBuf> {
    let candidates = [
        format!("tests/fixtures/{rel}"),
        format!("edge-worker/tests/fixtures/{rel}"),
        format!("../edge-worker/tests/fixtures/{rel}"),
    ];
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

fn assert_hash(rel: &str, expected: &str) {
    let path = match fixture_path(rel) {
        Some(p) => p,
        None => {
            eprintln!("SKIPPED: {rel} not present in this checkout");
            return;
        }
    };
    let bytes = std::fs::read(&path).expect("read fixture");
    let actual = sha256(&bytes);
    assert_eq!(
        actual, expected,
        "fixture {rel} hash mismatch.\n\
         expected: {expected}\n\
         actual:   {actual}\n\
         If you rebuilt the fixture, run `sha256sum {path:?}` and update \
         EXPECTED_*_HASH in tests/test_fixtures_match_source.rs.",
    );
}

#[test]
fn handler_fixture_intact() {
    assert_hash("handler.wasm", EXPECTED_HANDLER_HASH);
}

#[test]
fn legacy_test_handle_fixture_intact() {
    // The v0.1-era test-handle.wasm ships in-tree (committed in
    // `tests/fixtures/test-handle.wasm`) and is used by the existing
    // supervisor integration tests. We don't hash it here because
    // it's not regenerated — it's a frozen artifact. This test
    // simply asserts the file is present.
    let path = fixture_path("test-handle.wasm")
        .expect("legacy test-handle.wasm must be present in fixtures/");
    let bytes = std::fs::read(&path).expect("read");
    assert!(!bytes.is_empty(), "test-handle.wasm is empty");
}
