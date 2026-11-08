//! Opt-in, bounded request observations for startup experiments. Query strings are excluded.
use super::{now_ms, write_json};
use axum::{
    extract::Request,
    middleware::{self, Next},
    Router,
};
use serde::Serialize;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Default, Serialize)]
struct Count {
    requests: u64,
    first_at_ms: Option<i64>,
    last_at_ms: Option<i64>,
    last_status: Option<u16>,
}

/// Initialize the receipt before listening, then retain only three known endpoints.
pub(super) async fn instrument(router: Router, path: PathBuf) -> anyhow::Result<Router> {
    let counts: BTreeMap<String, Count> = ["GET /taskpane.html", "GET /config", "POST /ready"]
        .into_iter()
        .map(|key| (key.to_owned(), Count::default()))
        .collect();
    write_json(&path, &json!({"enabled":true,"requests":counts}), true).await?;
    let counts = Arc::new(Mutex::new(counts));
    Ok(
        router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let counts = counts.clone();
            let path = path.clone();
            async move {
                let key = format!("{} {}", request.method(), request.uri().path());
                let observed_at = now_ms();
                let response = next.run(request).await;
                let mut counts = counts.lock().await;
                if let Some(count) = counts.get_mut(&key) {
                    count.requests = count.requests.saturating_add(1);
                    count.first_at_ms.get_or_insert(observed_at);
                    count.last_at_ms = Some(observed_at);
                    count.last_status = Some(response.status().as_u16());
                    if let Err(error) =
                        write_json(&path, &json!({"enabled":true,"requests":*counts}), true).await
                    {
                        tracing::error!(%error, "Cannot persist probe request observation");
                    }
                }
                response
            }
        })),
    )
}
