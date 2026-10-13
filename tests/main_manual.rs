mod common;

use assert_cmd::Command;
use mockito::Server;
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

#[test]
fn main_manual_mocked() {
    let mut server = Server::new();
    let pub_date = common::recent_rfc3339(5);
    let hh_mock = server
        .mock("GET", "/search/vacancy/rss")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_header("content-type", "application/xml")
        .with_body(common::rss_feed(&[(
            &pub_date,
            "Rust dev",
            "http://example.com/vacancy/1",
        )]))
        .create();

    let tg_mock = server
        .mock("POST", "/bottoken/sendMessage")
        .expect(1)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .create();

    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");

    Command::cargo_bin("rust-hh-feed")
        .unwrap()
        .env("HH_BASE_URL", server.url())
        .env("TELEGRAM_API_BASE_URL", server.url())
        .env("TELEGRAM_BOT_TOKEN", "token")
        .env("TELEGRAM_CHAT_ID", "1")
        .env("POSTED_JOBS_PATH", &state_path)
        .assert()
        .success();

    let state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!(state["version"], 2);
    assert!(state["jobs"]["1"].as_str().is_some());
    assert!(state["last_successful_run_at"].as_str().is_some());

    hh_mock.assert();
    tg_mock.assert();
}
