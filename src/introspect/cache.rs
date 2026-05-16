use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceFingerprint {
    pub host_id_hashed: String,
    pub routeros_version: String,
    pub build_time: String,
    pub architecture: String,
    pub board_name: String,
    pub packages_hash: String,
    pub selected_protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheMeta {
    pub profile_name: String,
    pub cache_key: String,
    pub created_unix: u64,
    pub ttl_seconds: u64,
    pub fingerprint: DeviceFingerprint,
}

pub fn hash_host_id(host: &str) -> String {
    let mut hasher = DefaultHasher::new();
    host.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn compute_cache_key(profile_name: &str, fingerprint: &DeviceFingerprint) -> String {
    let mut hasher = DefaultHasher::new();
    profile_name.hash(&mut hasher);
    fingerprint.host_id_hashed.hash(&mut hasher);
    fingerprint.routeros_version.hash(&mut hasher);
    fingerprint.build_time.hash(&mut hasher);
    fingerprint.architecture.hash(&mut hasher);
    fingerprint.board_name.hash(&mut hasher);
    fingerprint.packages_hash.hash(&mut hasher);
    fingerprint.selected_protocol.hash(&mut hasher);
    format!("cache:{:016x}", hasher.finish())
}

pub fn new_cache_meta(
    profile_name: &str,
    created_unix: u64,
    ttl_seconds: u64,
    fingerprint: DeviceFingerprint,
) -> CacheMeta {
    let cache_key = compute_cache_key(profile_name, &fingerprint);
    CacheMeta {
        profile_name: profile_name.to_owned(),
        cache_key,
        created_unix,
        ttl_seconds,
        fingerprint,
    }
}

pub fn is_cache_stale(meta: &CacheMeta, now_unix: u64, current: &DeviceFingerprint) -> bool {
    if now_unix.saturating_sub(meta.created_unix) > meta.ttl_seconds {
        return true;
    }

    if &meta.fingerprint != current {
        return true;
    }

    let current_key = compute_cache_key(&meta.profile_name, current);
    meta.cache_key != current_key
}

#[cfg(test)]
mod tests {
    use super::{
        compute_cache_key, hash_host_id, is_cache_stale, new_cache_meta, DeviceFingerprint,
    };

    fn fingerprint(version: &str) -> DeviceFingerprint {
        DeviceFingerprint {
            host_id_hashed: hash_host_id("192.168.88.1"),
            routeros_version: version.to_owned(),
            build_time: "2026-01-01".to_owned(),
            architecture: "arm64".to_owned(),
            board_name: "RB5009".to_owned(),
            packages_hash: "pkg-hash".to_owned(),
            selected_protocol: "rest".to_owned(),
        }
    }

    #[test]
    fn host_hash_hides_plain_host_value() {
        let hashed = hash_host_id("192.168.88.1");
        assert!(!hashed.contains("192.168.88.1"));
        assert_eq!(hashed.len(), 16);
    }

    #[test]
    fn cache_key_changes_when_fingerprint_changes() {
        let v7 = fingerprint("7.15.3");
        let v6 = fingerprint("6.49.17");

        let key_v7 = compute_cache_key("home", &v7);
        let key_v6 = compute_cache_key("home", &v6);

        assert_ne!(key_v7, key_v6);
    }

    #[test]
    fn cache_is_stale_after_ttl() {
        let fp = fingerprint("7.15.3");
        let meta = new_cache_meta("home", 100, 10, fp.clone());

        assert!(is_cache_stale(&meta, 111, &fp));
    }

    #[test]
    fn cache_is_stale_when_fingerprint_changes() {
        let meta = new_cache_meta("home", 100, 600, fingerprint("7.15.3"));
        assert!(is_cache_stale(&meta, 200, &fingerprint("7.15.4")));
    }

    #[test]
    fn cache_is_fresh_when_ttl_and_fingerprint_match() {
        let fp = fingerprint("7.15.3");
        let meta = new_cache_meta("home", 100, 600, fp.clone());

        assert!(!is_cache_stale(&meta, 200, &fp));
    }
}
