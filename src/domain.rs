//! A domain name with a suffix known to the Public Suffix List.
use anyhow::bail;
use url::Host;

#[derive(Debug, Clone, PartialEq, Eq)]
/// A domain name with a suffix known to the Public Suffix List.
pub struct Domain {
    domain: String,
}

impl Domain {
    /// Construct from an arbitrary string.
    pub fn from_str(domain: &str) -> anyhow::Result<Self> {
        let name = match addr::parse_domain_name(domain) {
            Err(e) => bail!("Parsing domain name {} failed: {}", domain, e),
            Ok(name) => name,
        };
        if !name.has_known_suffix() {
            bail!(
                "The domain name {} has valid syntax, but its suffix is not in the Public Suffix List",
                domain
            )
        }
        let domain = name.as_str().to_owned();
        Ok(Self { domain })
    }

    /// Construct from [`url::Host::Domain`].
    pub fn from_host(host: &Host) -> anyhow::Result<Self> {
        match host {
            Host::Domain(domain) => Self::from_str(domain),
            Host::Ipv4(_) => bail!("The Host is an IPv4 address rather than a Domain"),
            Host::Ipv6(_) => bail!("The Host is an IPv6 address rather than a Domain"),
        }
    }
}

impl std::fmt::Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.domain)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn accepts_only_host_domain() {
        use url::Host;

        assert!(Domain::from_host(&Host::Ipv4(std::net::Ipv4Addr::new(127, 0, 0, 1))).is_err());
        assert!(
            Domain::from_host(&Host::Ipv6(std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)))
                .is_err()
        );
    }

    #[test]
    fn accepts_only_domains_with_known_suffixes() {
        use url::Host;

        // Full URLs
        assert!(Domain::from_host(&Host::Domain("http://example.org/hello".to_string())).is_err());
        assert!(Domain::from_host(&Host::Domain(
            "http://bar.example.org/goodbye#world".to_string()
        ))
        .is_err());
        assert!(Domain::from_host(&Host::Domain(
            "https://example.com/and/a/path?with=option".to_string()
        ))
        .is_err());
        assert!(Domain::from_host(&Host::Domain(
            "https://foo.example.com:81/and/a/path?with=option".to_string()
        ))
        .is_err());

        // IP addresses
        assert!(Domain::from_host(&Host::Domain("8.8.8.8".to_string())).is_err());
        assert!(Domain::from_host(&Host::Domain("127.0.0.1".to_string())).is_err());
        assert!(Domain::from_host(&Host::Domain("2001:4860:4860::8888".to_string())).is_err());
        assert!(Domain::from_host(&Host::Domain("[2001:4860:4860::8888]".to_string())).is_err());
        assert!(Domain::from_host(&Host::Domain("::1".to_string())).is_err());
        assert!(Domain::from_host(&Host::Domain("[::1]".to_string())).is_err());

        // Onion hidden services
        assert!(Domain::from_host(&Host::Domain("yzw45do3yrjfnbpr.onion".to_string())).is_ok());
        assert!(Domain::from_host(&Host::Domain(
            "zlzvfg5zcehs2t4qcm7woogyywfzwvrduqujsnehrjeg3tndn6a55nqd.onion".to_string()
        ))
        .is_ok());

        // I2P
        assert!(Domain::from_host(&Host::Domain("example.i2p".to_string())).is_err());

        // OpenNIC
        assert!(Domain::from_host(&Host::Domain("outdated.bbs".to_string())).is_err());
        // This one is dropped from OpenNIC and is coming to "real" DNS soon
        assert!(Domain::from_host(&Host::Domain("this.one.is.free".to_string())).is_ok());
    }

    #[test]
    fn accepts_only_str_domains_with_known_suffixes() {
        // Full URLs
        assert!(Domain::from_str("http://example.org/hello").is_err());
        assert!(Domain::from_str("http://bar.example.org/goodbye#world").is_err());
        assert!(Domain::from_str("https://example.com/and/a/path?with=option").is_err());
        assert!(Domain::from_str("https://foo.example.com:81/and/a/path?with=option").is_err());

        // IP addresses
        assert!(Domain::from_str("8.8.8.8").is_err());
        assert!(Domain::from_str("127.0.0.1").is_err());
        assert!(Domain::from_str("2001:4860:4860::8888").is_err());
        assert!(Domain::from_str("[2001:4860:4860::8888]").is_err());
        assert!(Domain::from_str("::1").is_err());
        assert!(Domain::from_str("[::1]").is_err());

        // Onion hidden services
        assert!(Domain::from_str("yzw45do3yrjfnbpr.onion").is_ok());
        assert!(
            Domain::from_str("zlzvfg5zcehs2t4qcm7woogyywfzwvrduqujsnehrjeg3tndn6a55nqd.onion")
                .is_ok()
        );

        // I2P
        assert!(Domain::from_str("example.i2p").is_err());

        // OpenNIC
        assert!(Domain::from_str("outdated.bbs").is_err());
        // This one is dropped from OpenNIC and is coming to "real" DNS soon
        assert!(Domain::from_str("this.one.is.free").is_ok());
    }

    #[test]
    fn what_addr_accepts_and_rejects() {
        use addr::parse_domain_name;

        // Full URLs
        assert!(parse_domain_name("http://example.org/hello").is_err());
        assert!(parse_domain_name("http://bar.example.org/goodbye#world").is_err());
        assert!(parse_domain_name("https://example.com/and/a/path?with=option").is_err());
        assert!(parse_domain_name("https://foo.example.com:81/and/a/path?with=option").is_err());

        // IP addresses
        assert!(parse_domain_name("8.8.8.8").is_err());
        assert!(parse_domain_name("127.0.0.1").is_err());
        assert!(parse_domain_name("2001:4860:4860::8888").is_err());
        assert!(parse_domain_name("[2001:4860:4860::8888]").is_err());
        assert!(parse_domain_name("::1").is_err());
        assert!(parse_domain_name("[::1]").is_err());

        // Onion hidden services
        assert!(parse_domain_name("yzw45do3yrjfnbpr.onion")
            .unwrap()
            .has_known_suffix());
        assert!(parse_domain_name(
            "zlzvfg5zcehs2t4qcm7woogyywfzwvrduqujsnehrjeg3tndn6a55nqd.onion"
        )
        .unwrap()
        .has_known_suffix());

        // I2P
        assert!(!parse_domain_name("example.i2p").unwrap().has_known_suffix());

        // OpenNIC
        assert!(!parse_domain_name("outdated.bbs")
            .unwrap()
            .has_known_suffix());
        // This one is dropped from OpenNIC and is coming to "real" DNS soon
        assert!(parse_domain_name("this.one.is.free")
            .unwrap()
            .has_known_suffix());
    }
}
