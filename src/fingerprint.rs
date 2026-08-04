use crate::types::Identity;
use std::collections::HashMap;

/// Counts occurrences of each identity — the multiset representation used
/// throughout matching. A profile is keyed and matched by this multiset,
/// never by connector name or ordering.
pub fn identity_counts(identities: &[Identity]) -> HashMap<Identity, u32> {
    let mut counts = HashMap::new();
    for id in identities {
        *counts.entry(id.clone()).or_insert(0) += 1;
    }
    counts
}

/// A blake3 hash of the canonically-sorted identity multiset, used as the
/// storage key for a profile. The hash is never the *only* representation:
/// callers must also persist the full identity list for inspectability.
pub fn fingerprint(identities: &[Identity]) -> String {
    let mut sorted: Vec<&Identity> = identities.iter().collect();
    sorted.sort_by(|a, b| (&a.make, &a.model, &a.serial).cmp(&(&b.make, &b.model, &b.serial)));

    let mut hasher = blake3::Hasher::new();
    for id in sorted {
        hasher.update(id.make.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(id.model.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(id.serial.as_bytes());
        hasher.update(&[0x1eu8]);
    }
    hasher.finalize().to_hex().to_string()
}

/// `sub`'s multiset is contained within `sup`'s — i.e. every identity in
/// `sub` appears in `sup` at least as many times.
pub fn is_sub_multiset(sub: &HashMap<Identity, u32>, sup: &HashMap<Identity, u32>) -> bool {
    sub.iter().all(|(id, count)| sup.get(id).copied().unwrap_or(0) >= *count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(make: &str, model: &str, serial: &str) -> Identity {
        Identity {
            make: make.to_string(),
            model: model.to_string(),
            serial: serial.to_string(),
        }
    }

    #[test]
    fn fingerprint_is_order_independent() {
        let a = vec![id("BOE", "0x0BC9", ""), id("DEL", "U2720Q", "ABC123")];
        let b = vec![id("DEL", "U2720Q", "ABC123"), id("BOE", "0x0BC9", "")];
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn fingerprint_differs_on_different_sets() {
        let a = vec![id("BOE", "0x0BC9", "")];
        let b = vec![id("BOE", "0x0BC9", ""), id("DEL", "U2720Q", "ABC123")];
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn fingerprint_distinguishes_duplicate_counts() {
        let one = vec![id("BOE", "0x0BC9", "")];
        let two = vec![id("BOE", "0x0BC9", ""), id("BOE", "0x0BC9", "")];
        assert_ne!(fingerprint(&one), fingerprint(&two));
    }

    #[test]
    fn sub_multiset_respects_counts() {
        let sub = identity_counts(&[id("A", "1", ""), id("A", "1", "")]);
        let sup_ok = identity_counts(&[id("A", "1", ""), id("A", "1", ""), id("B", "2", "")]);
        let sup_short = identity_counts(&[id("A", "1", ""), id("B", "2", "")]);
        assert!(is_sub_multiset(&sub, &sup_ok));
        assert!(!is_sub_multiset(&sub, &sup_short));
    }
}
