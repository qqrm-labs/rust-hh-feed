use chrono::{TimeZone, Utc};
use mockito::{Matcher, Server};
use rust_hh_feed::hh::HhClient;

fn rss_feed(items: &[(&str, &str, &str)]) -> String {
    let items_xml = items
        .iter()
        .map(|(pub_date, title, link)| {
            format!(
                "<item><pubDate>{pub_date}</pubDate><title>{title}</title><link>{link}</link><guid isPermaLink=\"true\">{link}</guid></item>"
            )
        })
        .collect::<Vec<_>>()
        .join("");

    format!(
        "<?xml version='1.0' encoding='utf-8'?><rss version=\"2.0\"><channel>{items_xml}</channel></rss>"
    )
}

#[tokio::test]
async fn fetch_jobs_parses_mock_rss_response() {
    let mut server = Server::new_async().await;
    let expected_user_agent = "rust-hh-feed/1.0 (qqrm@users.noreply.github.com)";
    let pub_date = (Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
    let mock = server
        .mock("GET", "/search/vacancy/rss")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("order_by".into(), "publication_time".into()),
            Matcher::UrlEncoded("search_field".into(), "name".into()),
            Matcher::UrlEncoded("text".into(), "rust".into()),
        ]))
        .match_header("user-agent", expected_user_agent)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_header("content-type", "application/xml")
        .with_body(rss_feed(&[(
            &pub_date,
            "Rust dev",
            "http://example.com/vacancy/1",
        )]))
        .create_async()
        .await;

    let client = HhClient::with_base_url(server.url());
    let jobs = client.fetch_jobs().await.unwrap();

    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job.id, "1");
    assert_eq!(job.name, "Rust dev");
    assert_eq!(job.url, "http://example.com/vacancy/1");
    assert!(job.snippet.is_none());

    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_jobs_between_filters_rss_items_by_requested_range() {
    let mut server = Server::new_async().await;
    let from = Utc.with_ymd_and_hms(2026, 4, 16, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 4, 20, 23, 59, 59).unwrap();

    let mock = server
        .mock("GET", "/search/vacancy/rss")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_header("content-type", "application/xml")
        .with_body(rss_feed(&[
            (
                "2026-04-20T12:29:41.773+03:00",
                "Middle Backend разработчик (Rust)",
                "http://example.com/vacancy/132235061",
            ),
            (
                "2026-04-15T14:43:52.856+03:00",
                "Backend-разработчик (RUST, C/C++)",
                "http://example.com/vacancy/132168741",
            ),
        ]))
        .create_async()
        .await;

    let client = HhClient::with_base_url(server.url());
    let jobs = client.fetch_jobs_between(from, to).await.unwrap();

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "132235061");

    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_jobs_fails_on_unsuccessful_status() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/search/vacancy/rss")
        .match_query(Matcher::Any)
        .with_status(403)
        .with_header("content-type", "application/xml")
        .with_body("<rss></rss>")
        .create_async()
        .await;

    let client = HhClient::with_base_url(server.url());
    let error = client.fetch_jobs().await.unwrap_err();
    let reqwest_error = error.downcast_ref::<reqwest::Error>().unwrap();

    assert_eq!(reqwest_error.status(), Some(reqwest::StatusCode::FORBIDDEN));

    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_jobs_fails_on_invalid_success_payload() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/search/vacancy/rss")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_header("content-type", "application/xml")
        .with_body("<rss><channel><item></rss>")
        .create_async()
        .await;

    let client = HhClient::with_base_url(server.url());
    let error = client.fetch_jobs().await.unwrap_err();

    assert!(error
        .to_string()
        .contains("failed to parse HeadHunter search RSS"));

    mock.assert_async().await;
}
