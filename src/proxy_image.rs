use std::net::IpAddr;

use url::Url;

pub fn hostname_matches_domain_patterns(hostname: &str, patterns: &[String]) -> bool {
    let hostname = hostname.trim().to_ascii_lowercase();
    if hostname.is_empty() {
        return false;
    }

    patterns.iter().any(|pattern| {
        let pattern = pattern.trim().to_ascii_lowercase();
        if pattern.is_empty() {
            return false;
        }

        if let Some(suffix) = pattern.strip_prefix('.') {
            hostname == suffix || hostname.ends_with(&pattern)
        } else {
            hostname == pattern
        }
    })
}

pub fn is_forbidden_fetch_target(url: &Url) -> bool {
    let hostname = url.host_str().unwrap_or_default().trim();
    if hostname.eq_ignore_ascii_case("localhost") {
        return true;
    }

    match hostname.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
        }
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
        Err(_) => false,
    }
}
