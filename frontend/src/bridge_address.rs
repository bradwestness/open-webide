//! Default transport address at the browser's platform boundary.
pub fn default_address(protocol: &str, host: &str, hostname: &str) -> String {
    if protocol == "https:" {
        format!("wss://{host}/bridge")
    } else {
        let hostname = if hostname.contains(':') && !hostname.starts_with('[') {
            format!("[{hostname}]")
        } else {
            hostname.to_string()
        };
        format!("ws://{hostname}:3001")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secure_origins_keep_proxy_port_and_http_keeps_lan_access() {
        assert_eq!(
            default_address("https:", "ide.home:8443", "ide.home"),
            "wss://ide.home:8443/bridge"
        );
        assert_eq!(
            default_address("http:", "192.168.1.20:3000", "192.168.1.20"),
            "ws://192.168.1.20:3001"
        );
        assert_eq!(
            default_address("http:", "localhost:3000", "localhost"),
            "ws://localhost:3001"
        );
        assert_eq!(
            default_address("http:", "[::1]:3000", "::1"),
            "ws://[::1]:3001"
        );
        assert_eq!(
            default_address("https:", "[::1]:8443", "[::1]"),
            "wss://[::1]:8443/bridge"
        );
    }
}
