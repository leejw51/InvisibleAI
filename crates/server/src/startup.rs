use causewaybay_shared::BackendAddress;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

fn print_endpoints(label: &str, url: &str) {
    println!("  {label}\n    Web URL       {url}\n    API URL       {url}/api");
}

pub async fn print_urls(address: SocketAddr) -> Vec<BackendAddress> {
    let port = address.port();
    let local = if address.ip().is_unspecified() {
        if address.is_ipv6() {
            format!("http://[::1]:{port}")
        } else {
            format!("http://127.0.0.1:{port}")
        }
    } else {
        format!("http://{address}")
    };
    let mut urls = vec![local];
    if address.ip().is_unspecified()
        && let Ok(output) = tokio::process::Command::new("/sbin/ifconfig")
            .output()
            .await
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(ip) = line
                .trim()
                .strip_prefix("inet ")
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<std::net::Ipv4Addr>().ok())
                && !ip.is_loopback()
                && !ip.is_link_local()
            {
                let url = format!("http://{ip}:{port}");
                if !urls.contains(&url) {
                    urls.push(url);
                }
            }
        }
    }
    let mut addresses = Vec::new();
    println!("\n  Invisible AI is ready\n\n  Listening on    {address}\n");
    for (index, url) in urls.iter().enumerate() {
        let label = if index == 0 && (address.ip().is_unspecified() || address.ip().is_loopback()) {
            "Local access"
        } else {
            "Network access"
        };
        print_endpoints(label, url);
        addresses.push(BackendAddress {
            label: label.into(),
            web_url: url.clone(),
        });
    }
    let mut vpn = None;
    for command in [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ] {
        // Only advertise other interfaces when the listener accepts them.
        if address.ip().is_loopback() {
            break;
        }
        let output = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::process::Command::new(command)
                .args(["status", "--json"])
                .kill_on_drop(true)
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = output
            && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        {
            let detected = tailscale_addresses(address, &v);
            if detected.is_empty() {
                continue;
            }
            vpn = detected
                .iter()
                .find(|a| a.label == "Tailscale IP")
                .or_else(|| detected.first())
                .map(|a| a.web_url.clone());
            for endpoint in detected {
                print_endpoints(&endpoint.label, &endpoint.web_url);
                addresses.push(endpoint);
            }
            break;
        }
    }
    if vpn.is_none() && address.ip().is_unspecified() {
        println!("  Tailscale URL   not detected (connect Tailscale to show your VPN URL)");
    }
    let remote = vpn.as_ref().or_else(|| urls.get(1)).unwrap_or(&urls[0]);
    println!("\n  API health      {remote}/api/health");
    println!("  Dashboard       {remote}/status");
    println!("  Tailwind CSS    {remote}/assets/app.css");
    println!(
        "\n  On your MacBook:\n  causewaybay client --backend {remote} --name MacBook\n\n  On your iPad, open {remote}\n"
    );
    addresses
}

// Advertise only addresses accepted by the listener, from this machine's running node.
fn tailscale_addresses(address: SocketAddr, status: &serde_json::Value) -> Vec<BackendAddress> {
    if status["BackendState"] != "Running" || address.ip().is_loopback() {
        return vec![];
    }
    let ips = status["TailscaleIPs"]
        .as_array()
        .or_else(|| status["Self"]["TailscaleIPs"].as_array());
    let mut ips = ips
        .into_iter()
        .flatten()
        .filter_map(|ip| ip.as_str()?.parse::<IpAddr>().ok())
        .filter(|ip| {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && (address.ip() == *ip
                    || (address.ip().is_unspecified() && (address.is_ipv6() || ip.is_ipv4())))
        })
        .collect::<Vec<_>>();
    ips.sort_by_key(|ip| (ip.is_ipv6(), *ip));
    ips.dedup();
    let mut endpoints = vec![];
    if !ips.is_empty()
        && let Some(dns) = status["Self"]["DNSName"]
            .as_str()
            .map(|v| v.trim_end_matches('.'))
            .filter(|v| {
                !v.is_empty()
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            })
    {
        endpoints.push(BackendAddress {
            label: "Tailscale DNS".into(),
            web_url: format!("http://{dns}:{}", address.port()),
        });
    }
    for ip in ips {
        endpoints.push(BackendAddress {
            label: "Tailscale IP".into(),
            web_url: format!("http://{}", SocketAddr::new(ip, address.port())),
        });
    }
    endpoints
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn publishes_self_dns_and_ipv4_with_actual_port() {
        let state = json!({"BackendState":"Running","TailscaleIPs":["fd7a:115c:a1e0::1","100.64.0.10"],"Self":{"DNSName":"studio.tailnet.ts.net."}});
        let endpoints = tailscale_addresses("0.0.0.0:3456".parse().unwrap(), &state);
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].web_url, "http://studio.tailnet.ts.net:3456");
        assert_eq!(endpoints[1].web_url, "http://100.64.0.10:3456");
        assert!(tailscale_addresses("127.0.0.1:3456".parse().unwrap(), &state).is_empty());
        assert!(tailscale_addresses("192.168.1.20:3456".parse().unwrap(), &state).is_empty());
        let mut stopped = state.clone();
        stopped["BackendState"] = json!("Stopped");
        assert!(tailscale_addresses("0.0.0.0:3456".parse().unwrap(), &stopped).is_empty());
    }
    #[test]
    fn ipv6_addresses_are_bracketed_and_invalid_dns_is_ignored() {
        let state = json!({"BackendState":"Running","Self":{"TailscaleIPs":["fd7a:115c:a1e0::1","invalid"],"DNSName":"bad;command"}});
        let endpoints = tailscale_addresses("[::]:3000".parse().unwrap(), &state);
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].web_url, "http://[fd7a:115c:a1e0::1]:3000");
    }
}
