/// FNV-1a, 64-bit.
///
/// Hand-rolled because these hashes are **persisted**: item guids derived from
/// content, and the fingerprint of the interest profile an item was scored
/// against. `DefaultHasher` makes no cross-version stability guarantee, so a
/// Rust upgrade could silently re-key every affected row.
pub fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// Short, stable fingerprint for a chunk of text.
pub fn fingerprint(input: &str) -> String {
    format!("{:016x}", fnv1a(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_input_always_fingerprints_the_same() {
        assert_eq!(fingerprint("hello"), fingerprint("hello"));
        assert_ne!(fingerprint("hello"), fingerprint("hello "));
        assert_eq!(fingerprint("").len(), 16);
    }
}
