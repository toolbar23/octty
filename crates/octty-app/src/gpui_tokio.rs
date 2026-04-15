use std::{future::Future, sync::Arc};

use gpui::{App, AppContext, AsyncApp, Global, Task};

pub fn init_from_runtime(cx: &mut App, runtime: Arc<tokio::runtime::Runtime>) {
    cx.set_global(GlobalTokio { runtime });
}

struct GlobalTokio {
    runtime: Arc<tokio::runtime::Runtime>,
}

impl Global for GlobalTokio {}

pub struct Tokio;

impl Tokio {
    pub fn spawn_result<Fut, R>(
        cx: &mut AsyncApp,
        future: Fut,
    ) -> anyhow::Result<Task<anyhow::Result<R>>>
    where
        Fut: Future<Output = anyhow::Result<R>> + Send + 'static,
        R: Send + 'static,
    {
        cx.read_global(|tokio: &GlobalTokio, cx| {
            let join_handle = tokio.runtime.spawn(future);
            cx.background_spawn(async move {
                match join_handle.await {
                    Ok(result) => result,
                    Err(error) => Err(error.into()),
                }
            })
        })
    }
}
