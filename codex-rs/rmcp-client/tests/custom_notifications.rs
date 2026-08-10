use std::io;
use std::sync::Arc;
use std::time::Duration;

use codex_rmcp_client::ElicitationAction;
use codex_rmcp_client::ElicitationResponse;
use codex_rmcp_client::InProcessTransportFactory;
use codex_rmcp_client::RmcpClient;
use futures::FutureExt;
use futures::future::BoxFuture;
use rmcp::ServerHandler;
use rmcp::ServiceExt;
use rmcp::model::ClientCapabilities;
use rmcp::model::CustomNotification;
use rmcp::model::Implementation;
use rmcp::model::InitializeRequestParams;
use rmcp::model::ProtocolVersion;
use rmcp::model::ServerNotification;
use serde_json::json;

struct CustomNotificationServer;

impl ServerHandler for CustomNotificationServer {
    async fn on_initialized(&self, context: rmcp::service::NotificationContext<rmcp::RoleServer>) {
        let _ = context
            .peer
            .send_notification(ServerNotification::CustomNotification(
                CustomNotification::new(
                    "notifications/codex/channel",
                    Some(json!({"text": "hello", "id": "m1"})),
                ),
            ))
            .await;
    }
}

struct CustomNotificationFactory;

impl InProcessTransportFactory for CustomNotificationFactory {
    fn open(&self) -> BoxFuture<'static, io::Result<tokio::io::DuplexStream>> {
        async move {
            let (server_transport, client_transport) = tokio::io::duplex(4096);
            tokio::spawn(async move {
                if let Ok(server) = CustomNotificationServer.serve(server_transport).await {
                    let _ = server.waiting().await;
                }
            });
            Ok(client_transport)
        }
        .boxed()
    }
}

fn init_params() -> InitializeRequestParams {
    InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new("codex-test", "0.0.0-test"),
    )
    .with_protocol_version(ProtocolVersion::V_2025_06_18)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn custom_server_notification_reaches_callback() -> anyhow::Result<()> {
    let client = RmcpClient::new_in_process_client(Arc::new(CustomNotificationFactory)).await?;
    let (notification_tx, mut notification_rx) = tokio::sync::mpsc::unbounded_channel();

    client
        .initialize_with_custom_notifications(
            init_params(),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
            Box::new(move |method, params| {
                let notification_tx = notification_tx.clone();
                async move {
                    let _ = notification_tx.send((method, params));
                }
                .boxed()
            }),
        )
        .await?;

    let notification = tokio::time::timeout(Duration::from_secs(5), notification_rx.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("custom notification channel closed"))?;
    assert_eq!(
        notification,
        (
            "notifications/codex/channel".to_string(),
            Some(json!({"text": "hello", "id": "m1"}))
        )
    );
    client.shutdown().await;
    Ok(())
}
