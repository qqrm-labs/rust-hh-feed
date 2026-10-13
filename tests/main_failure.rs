mod common;

use assert_cmd::Command;
use mockito::{Matcher, Server};
use std::fs;
use tempfile::tempdir;

#[test]
fn retry_keeps_confirmed_jobs_without_advancing_failed_fetch_watermark() {
    let mut server = Server::new();
    let first_pub_date = common::recent_rfc3339(5);
    let second_pub_date = common::recent_rfc3339(65);
    let hh_body = common::rss_feed(&[
        (&first_pub_date, "Rust dev", "http://example.com/vacancy/1"),
        (
            &second_pub_date,
            "Rust engineer",
            "http://example.com/vacancy/2",
        ),
    ]);
    let hh_mock = server
        .mock("GET", "/search/vacancy/rss")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_header("content-type", "application/xml")
        .with_body(hh_body)
        .create();

    let tg_ok = server
        .mock("POST", "/bottoken/sendMessage")
        .match_body(Matcher::Regex("http://example.com/vacancy/1".into()))
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .create();
    let tg_fail = server
        .mock("POST", "/bottoken/sendMessage")
        .match_body(Matcher::Regex("http://example.com/vacancy/2".into()))
        .with_status(500)
        .create();

    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    let initial_state = r#"{
  "version": 2,
  "last_successful_run_at": "2026-04-03T00:00:00Z",
  "jobs": {
    "existing": "2026-04-03"
  }
}"#;
    fs::write(&state_path, initial_state).unwrap();

    Command::cargo_bin("rust-hh-feed")
        .unwrap()
        .env("HH_BASE_URL", server.url())
        .env("TELEGRAM_API_BASE_URL", server.url())
        .env("TELEGRAM_BOT_TOKEN", "token")
        .env("TELEGRAM_CHAT_ID", "1")
        .env("POSTED_JOBS_PATH", &state_path)
        .assert()
        .failure();

    let content: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert!(content["jobs"]["1"].is_string());
    assert!(content["jobs"]["2"].is_null());
    assert_eq!(content["last_successful_run_at"], "2026-04-03T00:00:00Z");

    hh_mock.assert();
    tg_ok.assert();
    tg_fail.assert();

    // A second invocation must stop before network access while the send is pending.
    Command::cargo_bin("rust-hh-feed")
        .unwrap()
        .env("HH_BASE_URL", server.url())
        .env("TELEGRAM_API_BASE_URL", server.url())
        .env("TELEGRAM_BOT_TOKEN", "token")
        .env("TELEGRAM_CHAT_ID", "1")
        .env("POSTED_JOBS_PATH", &state_path)
        .assert()
        .failure();
    hh_mock.assert();
    tg_ok.assert();
    tg_fail.assert();
    // Reconcile the explicit mock rejection, then allow only the remaining job.
    fs::remove_file(state_path.with_extension("pending")).unwrap();

    let mut retry = Server::new();
    let feed = retry
        .mock("GET", "/search/vacancy/rss")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_body(common::rss_feed(&[
            (&first_pub_date, "Rust dev", "http://example.com/vacancy/1"),
            (
                &second_pub_date,
                "Rust engineer",
                "http://example.com/vacancy/2",
            ),
        ]))
        .create();
    let first = retry
        .mock("POST", "/bottoken/sendMessage")
        .match_body(Matcher::Regex("http://example.com/vacancy/1".into()))
        .expect(0)
        .create();
    let second = retry
        .mock("POST", "/bottoken/sendMessage")
        .match_body(Matcher::Regex("http://example.com/vacancy/2".into()))
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .create();
    Command::cargo_bin("rust-hh-feed")
        .unwrap()
        .env("HH_BASE_URL", retry.url())
        .env("TELEGRAM_API_BASE_URL", retry.url())
        .env("TELEGRAM_BOT_TOKEN", "token")
        .env("TELEGRAM_CHAT_ID", "1")
        .env("POSTED_JOBS_PATH", &state_path)
        .assert()
        .success();
    feed.assert();
    first.assert();
    second.assert();
}
