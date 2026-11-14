use rust_sync_proxy::provider::{ProviderKind, resolve_provider};

#[test]
fn grsai_provider_matches_root_and_subdomains() {
    assert_eq!(
        resolve_provider("https://api.grsai.com").kind(),
        ProviderKind::Grsai
    );
    assert_eq!(resolve_provider("https://api.grsai.com").name(), "grsai");
    assert_eq!(
        resolve_provider("https://grsai.com").kind(),
        ProviderKind::Grsai
    );
    assert_eq!(
        resolve_provider("https://sub.api.grsai.com").kind(),
        ProviderKind::Grsai
    );
}

#[test]
fn grsai_provider_rejects_suffix_tricks() {
    assert_eq!(
        resolve_provider("https://evilgrsai.com").kind(),
        ProviderKind::Transparent
    );
    assert_eq!(
        resolve_provider("https://grsai.com.evil.com").kind(),
        ProviderKind::Transparent
    );
}

#[test]
fn aiapidev_provider_matches_current_host() {
    assert_eq!(
        resolve_provider("https://www.aiapipro.vip").kind(),
        ProviderKind::Aiapidev
    );
    assert_eq!(
        resolve_provider("https://www.aiapipro.vip").name(),
        "aiapidev"
    );
}

#[test]
fn aiapidev_provider_rejects_old_hosts() {
    assert_eq!(
        resolve_provider("https://www.aiapidev.com").kind(),
        ProviderKind::Transparent
    );
    assert_eq!(
        resolve_provider("https://aiapidev.com").kind(),
        ProviderKind::Transparent
    );
}

#[test]
fn mai_token_provider_matches_only_the_documented_host() {
    let provider = resolve_provider("https://API.MAI-TOKEN.COM/v1");
    assert_eq!(provider.kind(), ProviderKind::MaiToken);
    assert_eq!(provider.name(), "mai-token");
    for base_url in [
        "https://mai-token.com",
        "https://other.mai-token.com",
        "https://api.mai-token.com.evil.example",
        "https://evilmai-token.com",
    ] {
        assert_eq!(resolve_provider(base_url).kind(), ProviderKind::Transparent);
    }
}

#[test]
fn tianyue_host_match_is_exact_and_case_insensitive() {
    use rust_sync_proxy::provider::is_tianyue_base_url;

    assert!(is_tianyue_base_url("https://API.TIANYUE.XYZ/v1"));
    for base_url in [
        "https://tianyue.xyz",
        "https://other.tianyue.xyz",
        "https://api.tianyue.xyz.evil.example",
        "not a url",
    ] {
        assert!(!is_tianyue_base_url(base_url));
    }
}

#[test]
fn unknown_or_invalid_base_url_uses_transparent_provider() {
    assert_eq!(
        resolve_provider("https://magic666.top").kind(),
        ProviderKind::Transparent
    );
    assert_eq!(
        resolve_provider("https://magic666.top").name(),
        "transparent"
    );
    assert_eq!(
        resolve_provider("not a url").kind(),
        ProviderKind::Transparent
    );
}
