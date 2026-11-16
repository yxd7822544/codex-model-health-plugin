//! 组合检测、定时任务和管理页面处理器
mod alerts;
mod account_groups;
pub mod domain;
pub mod engine;
mod host;
mod management;
mod runner;
mod schedules;
mod updates;

use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    client::{AuthorError, ComposedPlugin, Empty, PluginBuilder, TypedReply, methods},
};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, Semaphore};

#[derive(Default)]
pub(crate) struct RuntimeState {
    initialized: bool,
    recovery_reason: Option<String>,
}

pub(crate) struct Service {
    engine: engine::Engine,
    state: Mutex<RuntimeState>,
    execution: Semaphore,
    wake: Notify,
    updates: updates::UpdateChecker,
    schedules: Mutex<()>,
    alerts: alerts::Settings,
    host_api_base_url: String,
}
pub fn plugin(engine: engine::Engine) -> Result<ComposedPlugin, AuthorError> {
    plugin_with_configuration(engine, &serde_json::json!({}))
}
pub fn plugin_with_configuration(
    engine: engine::Engine,
    configuration: &serde_json::Value,
) -> Result<ComposedPlugin, AuthorError> {
    let service = Arc::new(Service {
        engine,
        state: Mutex::new(RuntimeState::default()),
        execution: Semaphore::new(1),
        wake: Notify::new(),
        updates: updates::UpdateChecker::default(),
        schedules: Mutex::new(()),
        alerts: alerts::Settings::from_configuration(configuration),
        host_api_base_url: configuration["hostApiBaseUrl"].as_str().unwrap_or("http://127.0.0.1:8080").to_owned(),
    });
    let maintenance = Arc::clone(&service);
    PluginBuilder::from_json(include_bytes!("../../plugin.json"))?
        .on(methods::RECONCILE, move |call| {
            let service = Arc::clone(&maintenance);
            async move {
                runner::reconcile(&service, &call.host).await?;
                Ok(TypedReply::new(Empty {}))
            }
        })?
        .management(management::registration(), move |call| {
            let service = Arc::clone(&service);
            async move { management::handle(&service, call).await }
        })?
        .build()
}
pub(crate) fn invalid(message: impl Into<String>) -> PluginFault {
    PluginFault::new(ErrorCode::InvalidInput, message)
}
pub fn call_budgets() -> gateway_plugin_sdk::CallBudgets {
    [(gateway_plugin_sdk::Stage::Maintenance, 3_600_000)].into()
}
pub(crate) fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos()
}
pub(crate) fn now_ms() -> u64 {
    (now_nanos() / 1_000_000) as u64
}
