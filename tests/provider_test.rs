use rust_sync_proxy::provider::{ProviderKind, resolve_provider};

#[test]
fn grsai_provider_matches_root_and_subdomains() {
    assert_eq!(
        resolve_provider("https://api.grsai.com").kind,
        ProviderKind::Grsai
    );
    assert_eq!(
        resolve_provider("https://grsai.com").kind,
        ProviderKind::Grsai
    );
    assert_eq!(
        resolve_provider("https://sub.api.grsai.com").kind,
        ProviderKind::Grsai
    );
}

#[test]
fn grsai_provider_rejects_suffix_tricks() {
    assert_eq!(
        resolve_provider("https://evilgrsai.com").kind,
        ProviderKind::Transparent
    );
    assert_eq!(
        resolve_provider("https://grsai.com.evil.com").kind,
        ProviderKind::Transparent
    );
}

#[test]
fn aiapidev_provider_matches_existing_hosts() {
    assert_eq!(
        resolve_provider("https://aiapidev.com").kind,
        ProviderKind::Aiapidev
    );
    assert_eq!(
        resolve_provider("https://www.aiapidev.com").kind,
        ProviderKind::Aiapidev
    );
}

#[test]
fn unknown_or_invalid_base_url_uses_transparent_provider() {
    assert_eq!(
        resolve_provider("https://magic666.top").kind,
        ProviderKind::Transparent
    );
    assert_eq!(
        resolve_provider("not a url").kind,
        ProviderKind::Transparent
    );
}
