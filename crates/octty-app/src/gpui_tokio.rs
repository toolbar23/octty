use std::future::Future;

use gpui::{App, AppContext, AsyncApp, Global, Task};

pub fn init_from_handle(cx: &mut App, handle: tokio::runtime::Handle) {
    cx.set_global(GlobalTokio { handle });
}

struct GlobalTokio {
    handle: tokio::runtime::Handle,
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
            let join_handle = tokio.handle.spawn(future);
            cx.background_spawn(async move {
                match join_handle.await {
                    Ok(result) => result,
                    Err(error) => Err(error.into()),
                }
            })
        })
    }
}
