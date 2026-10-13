mod common;

use assert_cmd::Command;
use mockito::Server;
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

#[test]
fn main_ignores_jobs_with_rust_only_in_description() {
    let mut server = Server::new();
    let pub_date = common::recent_rfc3339(5);
    let hh_body = common::rss_feed(&[(&pub_date, "Backend dev", "http://example.com/vacancy/1")]);
    let hh_mock = server
        .mock("GET", "/search/vacancy/rss")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_header("content-type", "application/xml")
        .with_body(hh_body)
        .create();

    let tg_mock = server
        .mock("POST", "/bottoken/sendMessage")
        .expect(0)
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
    assert!(state["jobs"]
        .as_object()
        .is_some_and(|jobs| !jobs.contains_key("1")));
    assert!(state["last_successful_run_at"].as_str().is_some());

    hh_mock.assert();
    tg_mock.assert();
}
