use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use futures::stream::StreamExt;
use futures::SinkExt;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Push hub modeled after SiYuan's util/websocket.go: one channel,
/// JSON envelopes `{cmd, code, msg, data}` broadcast to all clients.
pub static HUB: Lazy<Arc<broadcast::Sender<String>>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(256);
    Arc::new(tx)
});

pub fn broadcast(cmd: &str, data: Value) {
    let env = json!({
        "cmd": cmd,
        "code": 0,
        "msg": "",
        "data": data,
    });
    let _ = HUB.send(serde_json::to_string(&env).unwrap_or_default());
}

/// Common push commands (SiYuan naming for frontend familiarity)
pub fn push_filetree_changed() {
    broadcast("filetree-changed", json!({}));
}
pub fn push_doc_reindexed(path: &str) {
    broadcast("doc-reindexed", json!({"path": path}));
}
pub fn push_vt_changed() {
    broadcast("vt-changed", json!({}));
}
pub fn push_progress(done: u64, total: u64) {
    broadcast("progress", json!({"done": done, "total": total}));
}

pub async fn ws_handler(upgrade: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    upgrade.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = HUB.subscribe();

    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if sender.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(t) = msg {
                // Client pings: respond inline (SiYuan cmd ping)
                if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(&t) {
                    if o.get("cmd").and_then(|c| c.as_str()) == Some("ping") {
                        let _ = serde_json::to_string(&json!({"cmd":"ping","code":0,"msg":"","data":"pong"}));
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}
