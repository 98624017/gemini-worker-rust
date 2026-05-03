use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Grsai,
    Aiapidev,
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provider {
    pub kind: ProviderKind,
    pub name: &'static str,
}

pub fn resolve_provider(base_url: &str) -> Provider {
    if is_grsai_base_url(base_url) {
        return Provider {
            kind: ProviderKind::Grsai,
            name: "grsai",
        };
    }
    if is_aiapidev_base_url(base_url) {
        return Provider {
            kind: ProviderKind::Aiapidev,
            name: "aiapidev",
        };
    }
    Provider {
        kind: ProviderKind::Transparent,
        name: "transparent",
    }
}

pub fn is_grsai_base_url(raw: &str) -> bool {
    let Some(host) = parse_host(raw) else {
        return false;
    };
    host == "grsai.com" || host.ends_with(".grsai.com")
}

pub fn is_aiapidev_base_url(raw: &str) -> bool {
    let Some(host) = parse_host(raw) else {
        return false;
    };
    matches!(host.as_str(), "aiapidev.com" | "www.aiapidev.com")
}

fn parse_host(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}
