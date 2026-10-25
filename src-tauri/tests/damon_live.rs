//! Live E2E against a real damond at ws://127.0.0.1:9470 (protocol v2).
//!
//! Skipped unless `DAMON_LIVE=1` is set, so the default suite never requires a
//! daemon. Run with:
//!   DAMON_LIVE=1 cargo test --test damon_live -- --nocapture
//! Requires: damond running + at least one agent backend (claude) logged in.

use confluence_studio_lib::damon::DamonClient;

fn live_enabled() -> bool {
    std::env::var("DAMON_LIVE").ok().as_deref() == Some("1")
}

#[tokio::test]
async fn live_connect_create_turn_stream_complete() {
    if !live_enabled() {
        eprintln!("skipping: set DAMON_LIVE=1 with a running damond");
        return;
    }
    // 1. connect (same path the app's ensure_damon uses)
    let client = DamonClient::connect("ws://127.0.0.1:9470/ws", None)
        .await
        .expect("connect to real damond");
    let mut events = client.subscribe_events();

    // 2. session.create with a real backend
    let session = client
        .create_session(Some("claude"), None)
        .await
        .expect("session.create");
    eprintln!("session_id={} backend={}", session.session_id, session.backend);

    // 3. turn.start — resolves when the turn ends; stream rides the broadcast
    let result = client
        .turn_start(&session.session_id, "Reply with exactly: APP-LIVE-OK")
        .await
        .expect("turn.start");
    assert_eq!(result.stop_reason, "completed", "turn result: {result:?}");

    // 이벤트는 응답보다 먼저 broadcast됐다 — 잠깐 기다렸다가 drain.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mut assistant = String::new();
    let mut saw_turn_started = false;
    let mut saw_completed = false;
    while let Ok(push) = events.try_recv() {
        if push.session_id.as_deref() != Some(session.session_id.as_str()) {
            continue;
        }
        let Some(data) = push.data else { continue };
        match data.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "turn_started" => saw_turn_started = true,
            "timeline" => {
                if data.get("kind").and_then(|v| v.as_str()) == Some("assistant_message") {
                    if let Some(t) = data.get("text").and_then(|v| v.as_str()) {
                        assistant.push_str(t);
                    }
                }
            }
            "turn_completed" => saw_completed = true,
            _ => {}
        }
    }
    eprintln!(
        "streamed={assistant:?} turn_started={saw_turn_started} turn_completed={saw_completed}"
    );
    assert!(
        assistant.contains("APP-LIVE-OK"),
        "assistant stream must contain APP-LIVE-OK"
    );
    assert!(saw_turn_started, "turn_started event missing");
    assert!(saw_completed, "turn_completed event missing");

    // 4. cleanup
    let _ = client.delete_session(&session.session_id).await;
    client.close().await;
}

#[tokio::test]
async fn live_list_sessions_parses_real_created_at() {
    if !live_enabled() {
        eprintln!("skipping: set DAMON_LIVE=1 with a running damond");
        return;
    }
    let client = DamonClient::connect("ws://127.0.0.1:9470/ws", None)
        .await
        .expect("connect");
    let sessions = client.list_sessions(Some(10)).await.expect("session.list");
    eprintln!("sessions={}", sessions.len());
    for s in sessions.iter().take(5) {
        eprintln!(
            "  {} backend={:?} createdAt={:?}",
            s.session_id, s.backend, s.created_at
        );
    }
    client.close().await;
}
