//! Integration tests for RustRelay.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_health() {
        let client = reqwest::Client::new();
        let resp = client
            .get("http://127.0.0.1:8080/api/health")
            .send()
            .await
            .expect("Server not running");
        assert!(resp.status().is_success());
        assert_eq!(resp.text().await.unwrap(), "OK");
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_stats() {
        let client = reqwest::Client::new();
        let resp = client
            .get("http://127.0.0.1:8080/api/stats")
            .send()
            .await
            .expect("Server not running");
        assert!(resp.status().is_success());
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["node_id"].is_string());
        assert!(body["active_connections"].is_number());
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_websocket_connect() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message;

        let (ws, _) = connect_async("ws://127.0.0.1:8080/ws")
            .await
            .expect("Failed to connect");
        let (mut sink, mut stream) = ws.split();

        sink.send(Message::Text("token_alice".to_string()))
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("Timeout")
            .expect("Stream ended")
            .expect("WS error");

        if let Message::Text(text) = msg {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(json["t"], "READY");
            assert_eq!(json["d"]["user"]["username"], "alice");
        } else {
            panic!("Expected text message");
        }
    }

    #[tokio::test]
    #[ignore = "requires running server"]
    async fn test_heartbeat() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message;

        let (ws, _) = connect_async("ws://127.0.0.1:8080/ws").await.unwrap();
        let (mut sink, mut stream) = ws.split();

        sink.send(Message::Text("token_alice".to_string()))
            .await
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.next()).await;

        let hb = serde_json::json!({"op": "heartbeat", "d": {"seq": 42}});
        sink.send(Message::Text(hb.to_string())).await.unwrap();

        let ack = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("Timeout")
            .expect("Stream ended")
            .expect("WS error");

        if let Message::Text(text) = ack {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(json["t"], "HEARTBEAT_ACK");
            assert_eq!(json["d"]["seq"], 42);
        } else {
            panic!("Expected text message");
        }
    }
}