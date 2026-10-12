use causewaybay_server::{AppState, connections};

#[test]
fn dashboard_counts_live_clients_and_late_disconnect_does_not_hide_reconnected_tab()
-> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let s = AppState::open(dir.path().into(), false, None)?;
    s.db.client_hello("macbook", "My MacBook", "macos")?;
    s.db.web_connect("ipad-tab", "Safari · iPad", "iPad", "first-socket")?;
    let d = connections::snapshot(&s)?;
    assert_eq!((d.rust_connected, d.web_connected), (1, 1));
    s.db.web_connect("ipad-tab", "Safari · iPad", "iPad", "second-socket")?;
    s.db.web_disconnect("ipad-tab", "first-socket")?;
    let d = connections::snapshot(&s)?;
    assert_eq!(d.web_clients.len(), 1);
    assert_eq!(d.web_connected, 1);
    s.db.web_disconnect("ipad-tab", "second-socket")?;
    s.db.release_captures("macbook")?;
    let d = connections::snapshot(&s)?;
    assert_eq!((d.rust_connected, d.web_connected), (0, 0));
    assert_eq!(d.web_clients[0].name, "Safari · iPad");
    Ok(())
}

#[test]
fn stale_heartbeats_and_backend_restart_do_not_report_clients_as_connected() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    {
        let s = AppState::open(dir.path().into(), false, None)?;
        s.db.client_hello("macbook", "My MacBook", "macos")?;
        s.db.web_connect("tab", "Browser", "macOS", "socket")?;
        s.db.0
            .lock()
            .unwrap()
            .execute("UPDATE web_clients SET last_seen=last_seen-36", [])?;
        assert_eq!(connections::snapshot(&s)?.web_connected, 0);
        s.db.web_heartbeat("tab", "socket")?;
        assert_eq!(connections::snapshot(&s)?.web_connected, 1);
    }
    let s = AppState::open(dir.path().into(), false, None)?;
    let d = connections::snapshot(&s)?;
    assert_eq!((d.rust_connected, d.web_connected), (0, 0));
    assert_eq!(d.web_clients.len(), 1);
    assert_eq!(d.rust_clients.len(), 1);
    Ok(())
}
