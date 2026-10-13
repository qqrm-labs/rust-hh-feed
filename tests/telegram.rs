use mockito::Server;
use reqwest::StatusCode;
use rust_hh_feed::telegram::TelegramBot;

#[tokio::test]
async fn send_message_returns_status() {
    let mut server = Server::new_async().await;
    let token = "test".to_string();
    let chat_id = "123".to_string();
    let path = format!("/bot{token}/sendMessage");
    let mock = server
        .mock("POST", path.as_str())
        .with_status(200)
        .with_body(r#"{"ok":true}"#)
        .with_header("content-type", "application/json")
        .create_async()
        .await;

    let bot = TelegramBot::with_base_url(token, chat_id, server.url());
    let status = bot.send_message("hi").await.unwrap();

    assert_eq!(status, StatusCode::OK);
    mock.assert_async().await;
}

#[tokio::test]
async fn send_message_fails_on_unsuccessful_status() {
    let mut server = Server::new_async().await;
    let token = "test".to_string();
    let chat_id = "123".to_string();
    let path = format!("/bot{token}/sendMessage");
    let mock = server
        .mock("POST", path.as_str())
        .with_status(500)
        .create_async()
        .await;

    let bot = TelegramBot::with_base_url(token, chat_id, server.url());
    let error = bot.send_message("hi").await.unwrap_err();

    assert!(error.to_string().contains("500"));
    mock.assert_async().await;
}
