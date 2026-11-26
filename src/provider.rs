use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Grsai,
    Aiapidev,
    MaiToken,
    Transparent,
}

impl ProviderKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Grsai => "grsai",
            Self::Aiapidev => "aiapidev",
            Self::MaiToken => "mai-token",
            Self::Transparent => "transparent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provider {
    kind: ProviderKind,
}

impl Provider {
    pub fn kind(self) -> ProviderKind {
        self.kind
    }

    pub fn name(self) -> &'static str {
        self.kind.name()
    }
}

pub fn resolve_provider(base_url: &str) -> Provider {
    if is_grsai_base_url(base_url) {
        return Provider {
            kind: ProviderKind::Grsai,
        };
    }
    if is_aiapidev_base_url(base_url) {
        return Provider {
            kind: ProviderKind::Aiapidev,
        };
    }
    if parse_host(base_url).as_deref() == Some("api.mai-token.com") {
        return Provider {
            kind: ProviderKind::MaiToken,
        };
    }
    Provider {
        kind: ProviderKind::Transparent,
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
    host == "www.aiapipro.vip"
}

pub fn is_tianyue_base_url(raw: &str) -> bool {
    parse_host(raw).as_deref() == Some("api.tianyue.xyz")
}

pub(crate) fn is_ai666_base_url(raw: &str) -> bool {
    parse_host(raw).as_deref() == Some("us.ai666.live")
}

fn parse_host(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}
