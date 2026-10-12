use crate::i18n::{t, tf};
use crate::{
    api::{self, ConnectionState},
    icons::icon,
};
use causewaybay_shared::{BackendAddress, CaptureClient, Dashboard};
use yew::prelude::*;

pub struct Actions {
    pub rename: Callback<CaptureClient>,
    pub default: Callback<String>,
    pub copy: Callback<String>,
}

pub fn view(
    data: Option<&Dashboard>,
    state: ConnectionState,
    is_admin: bool,
    owner: Option<&str>,
    actions: Actions,
    default_device: &str,
) -> Html {
    let connected = state == ConnectionState::Connected;
    let base = web_sys::window()
        .unwrap()
        .location()
        .origin()
        .unwrap_or_default();
    let empty = Dashboard::default();
    let d = data.unwrap_or(&empty);
    let commands = capture_commands(&d.addresses, &base, owner);
    let count = |n: usize| {
        if !connected {
            "—".into()
        } else if data.is_none() {
            "…".into()
        } else {
            n.to_string()
        }
    };
    let mut addresses = d.addresses.clone();
    if addresses.is_empty() {
        addresses.push(causewaybay_shared::BackendAddress {
            label: "Browser access".into(),
            web_url: base.clone(),
        });
    }
    html! {
        <section class="connection-dashboard" aria-label={t("Connection dashboard")}>
            <div class="page-intro"><div><div class="eyebrow">{icon("dashboard")}{t("YOUR DEVICES, TOGETHER")}</div><h1>{t("Your connected devices.")}</h1><p>{t("See your backend, capture computers, and open web clients in one place.")}</p></div></div>
            <div class={classes!("connection-notice",(!connected).then_some("waiting"))} aria-live="polite">
                {icon(if connected {"check"} else {"clock"})}
                <span>{t(match state { ConnectionState::Connected=>"Connected to your backend. This dashboard updates automatically.", ConnectionState::Connecting=>"Connecting to your backend…", ConnectionState::Reconnecting=>"Connection lost. Waiting 2 seconds between retries. Reconnection is automatic." })}</span>
            </div>
            <section class="device-setup" id="device-setup" aria-label={t("Connect a capture device")}><h2>{t("Connect a capture device")}</h2><p>{t("Run this command on the computer whose screen you want to capture.")}</p>{for commands.iter().map(|(label,command)|html!{<div class="capture-command" role="group" aria-label={t(label)}><strong class="capture-command-label">{t(label)}</strong><div class="copyable-command"><code aria-label={t("Capture client command")}>{command}</code>{copy_button("Copy capture command",command,&actions.copy)}</div></div>})}<p>{if owner.is_some(){t("This connects the computer to your wallet. Allow Screen Recording when macOS asks.")}else{t("Allow Screen Recording when macOS asks. The client saves this connection and reconnects automatically.")}}</p><p class="device-enrollment-help">{t("For TextEdit Auto, stop the existing capture CLI on this Mac, then run the account command above. Your account is saved for the next launch.")}</p><a class="text-button" href="/">{t("Return to New thought")}{icon("arrow")}</a></section>
            <div class="dashboard-stats">
                <article class="dashboard-stat"><span>{icon("monitor")}{t("Backend")}</span><strong data-testid="backend-status">{t(state.label())}</strong><small>{if connected {tf("Running for {duration}", &[("duration", duration(d.uptime_seconds))])}else{t("Waiting for the backend")}}</small></article>
                <article class="dashboard-stat"><span>{icon("screen")}{t("Rust clients")}</span><strong data-testid="rust-connected">{count(d.rust_connected)}</strong><small>{t("Connected capture computers")}</small></article>
                <article class="dashboard-stat"><span>{icon("home")}{t("Web clients")}</span><strong data-testid="web-connected">{count(d.web_connected)}</strong><small>{t("Connected browser tabs")}</small></article>
            </div>
            <div class="dashboard-devices">
                {clients("Rust capture clients", "screen", &d.rust_clients, connected, "Start the Rust client on your MacBook to connect it here.", if is_admin {Some(&actions.rename)}else{None}, Some((default_device, &actions.default)))}
                {clients("Web clients", "home", &d.web_clients, connected, "Open this workspace in a browser to connect.", None, None)}
            </div>
            <section class="dashboard-endpoints" aria-label={t("Backend addresses")}>
                <h2>{t("Backend addresses")}</h2>
                {for addresses.iter().enumerate().map(|(index,endpoint)|html!{<section class="backend-address" data-testid="backend-address" aria-labelledby={format!("endpoint-{index}")}>
                    <h3 id={format!("endpoint-{index}")}>{t(&endpoint.label)}</h3>
                    <dl>{for [("Web URL",endpoint.web_url.clone()),("API URL",format!("{}/api",endpoint.web_url))].iter().map(|(label,url)|html!{
                        <div><dt>{t(label)}</dt><dd><a href={url.clone()}>{url}</a></dd>{copy_button(&format!("Copy {label}"),url,&actions.copy)}</div>
                    })}</dl>
                </section>})}
                <p>{if connected {tf("Invisible AI {version} · JSON WebSocket connection", &[("version", d.server_version.clone())])}else{t("Last known device information is shown until the connection returns.")}}</p>
            </section>
        </section>
    }
}

fn clients(
    title: &str,
    ico: &str,
    clients: &[CaptureClient],
    connected: bool,
    empty: &str,
    rename: Option<&Callback<CaptureClient>>,
    default: Option<(&str, &Callback<String>)>,
) -> Html {
    let own_id = api::client_id();
    let collapse_offline = title == "Web clients";
    let recent = clients.iter().filter(|c| !c.online).collect::<Vec<_>>();
    let row = |c: &CaptureClient| {
        let online = connected && c.online;
        let seen = (js_sys::Date::now() / 1000.) as i64 - c.last_seen;
        html! {<li class="device-row" data-client-id={c.id.clone()}><span class="device-avatar">{icon(ico)}</span><div class="device-info"><strong>{&c.name}</strong>if default.is_some_and(|(id,_)|id==c.id){<span class="device-default">{t("Default")}</span>}<small>{&c.platform}if default.is_some() && c.platform.eq_ignore_ascii_case("macos"){<span class="device-textedit-access">{" · "}{t(match c.textedit {causewaybay_shared::TextEditAvailability::Ready=>"TextEdit ready",causewaybay_shared::TextEditAvailability::NeedsEnrollment=>"Screen capture only",_=>"Reconnect CLI"})}</span>}if c.id==own_id{<span class="this-browser">{t("This tab")}</span>}</small><span class="device-seen">{if c.last_seen==0 {t("Currently offline")}else if online&&seen<15{t("Seen just now")}else{tf("Last seen {duration} ago", &[("duration", duration(seen.max(0) as u64))])}}</span></div><span class={classes!("presence-badge",online.then_some("online"))}>{if !connected{t("Unknown")}else if online{t("Connected")}else{t("Offline")}}</span><div class="device-actions">{default.filter(|_|c.can_capture).map(|(id,callback)| { let callback=callback.clone();let client=c.id.clone();html!{<button class="text-button" disabled={id==c.id} aria-label={tf("Use {name} as default", &[("name",c.name.clone())])} onclick={Callback::from(move |_|callback.emit(client.clone()))}>{if id==c.id{t("Default device")}else{t("Make default")}}</button>} }).unwrap_or_default()}{rename.map(|callback| { let callback=callback.clone(); let client=c.clone(); html!{<button class="text-button" disabled={!connected} aria-label={tf("Rename {name}", &[("name", c.name.clone())])} onclick={Callback::from(move |_|callback.emit(client.clone()))}>{t("Rename")}</button>} }).unwrap_or_default()}</div></li>}
    };
    html! {<section class="device-panel" aria-label={t(title)}><div class="device-panel-heading"><h2>{icon(ico)}{t(title)}</h2><span>{tf("{count} known", &[("count", clients.len().to_string())])}</span></div>
        if default.is_some(){<p class="device-preference-hint">{t("Your default device and capture choices are saved on this browser for your wallet.")}</p>}
        if clients.is_empty(){<p class="device-empty">{t(empty)}</p>}
        <ul>{for clients.iter().filter(|c|!collapse_offline||c.online).map(&row)}</ul>
        if collapse_offline && !recent.is_empty(){<details class="device-history"><summary>{tf("Recently disconnected · {count}", &[("count", recent.len().to_string())])}</summary><ul>{for recent.into_iter().take(5).map(row)}</ul></details>}
    </section>}
}
fn duration(seconds: u64) -> String {
    if seconds >= 86400 {
        tf("{count}d", &[("count", (seconds / 86400).to_string())])
    } else if seconds >= 3600 {
        tf(
            "{hours}h {minutes}m",
            &[
                ("hours", (seconds / 3600).to_string()),
                ("minutes", (seconds % 3600 / 60).to_string()),
            ],
        )
    } else if seconds >= 60 {
        tf("{count}m", &[("count", (seconds / 60).to_string())])
    } else {
        tf("{count}s", &[("count", seconds.to_string())])
    }
}

fn copy_button(label: &str, text: &str, callback: &Callback<String>) -> Html {
    let text = text.to_string();
    html! {<button class="text-button copy-address" aria-label={t(label)} onclick={callback.reform(move |_|text.clone())}>{icon("copy")}{t("Copy")}</button>}
}
/// Advertise only discovered Tailscale endpoints; fall back to LAN/local/browser access.
fn capture_commands(
    addresses: &[BackendAddress],
    fallback: &str,
    owner: Option<&str>,
) -> Vec<(String, String)> {
    let mut endpoints = vec![];
    for label in ["Tailscale IP", "Tailscale DNS"] {
        if let Some(address) = addresses
            .iter()
            .find(|a| a.label == label && !a.web_url.trim().is_empty())
        {
            endpoints.push((address.label.as_str(), address.web_url.as_str()));
        }
    }
    if endpoints.is_empty() {
        let local = ["Network access", "Local access"].iter().find_map(|label| {
            addresses
                .iter()
                .find(|a| a.label == *label && !a.web_url.trim().is_empty())
        });
        endpoints.push(
            local
                .map(|a| (a.label.as_str(), a.web_url.as_str()))
                .unwrap_or(("Browser access", fallback)),
        );
    }
    let wallet = owner
        .map(|address| format!(" {address}"))
        .unwrap_or_default();
    endpoints
        .into_iter()
        .map(|(label, url)| (label.into(), format!("./bin/client {url}{wallet}")))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_include_discovered_tailscale_dns_and_preserve_wallet() {
        let mut addresses: Vec<_> = [
            ("Local access", "http://127.0.0.1:3000"),
            ("Network access", "http://192.168.1.20:3000"),
            ("Tailscale DNS", "http://studio.tailnet.ts.net:3000"),
            ("Tailscale IP", "http://100.64.0.10:3000"),
        ]
        .into_iter()
        .map(|(label, url)| BackendAddress {
            label: label.into(),
            web_url: url.into(),
        })
        .collect();
        let wallet = "0x1111111111111111111111111111111111111111";
        let commands = capture_commands(&addresses, "http://browser:3000", Some(wallet));
        assert_eq!(
            commands,
            vec![
                (
                    "Tailscale IP".into(),
                    format!("./bin/client http://100.64.0.10:3000 {wallet}")
                ),
                (
                    "Tailscale DNS".into(),
                    format!("./bin/client http://studio.tailnet.ts.net:3000 {wallet}")
                ),
            ]
        );
        addresses.pop(); // DNS alone is shown once.
        assert_eq!(
            capture_commands(&addresses, "", None),
            vec![(
                "Tailscale DNS".into(),
                "./bin/client http://studio.tailnet.ts.net:3000".into()
            )]
        );
        addresses.pop(); // No Tailscale discovery: use the LAN address.
        assert_eq!(
            capture_commands(&addresses, "", None),
            vec![(
                "Network access".into(),
                "./bin/client http://192.168.1.20:3000".into()
            )]
        );
        addresses.push(BackendAddress {
            label: "Tailscale DNS".into(),
            web_url: String::new(),
        });
        assert_eq!(capture_commands(&addresses, "", None).len(), 1);
        assert_eq!(
            capture_commands(&[], "http://browser:3000", None),
            vec![(
                "Browser access".into(),
                "./bin/client http://browser:3000".into()
            )]
        );
    }
}
