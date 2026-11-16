//! 启动插件会话并读取宿主配置
#[cfg(feature = "bundled-engine")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use gateway_plugin_sdk::client::{PluginSession, SessionConfig};
    let mut session = PluginSession::accept(
        tokio::io::stdin(),
        tokio::io::stdout(),
        SessionConfig::default(),
    )
    .await?;
    session.set_call_budgets(codex_model_health_plugin::call_budgets())?;
    if session.handshake().plugin_id != "local.model-health" {
        return Err("插件身份不匹配".into());
    }
    let plugin = codex_model_health_plugin::plugin_with_configuration(
        codex_model_health_plugin::engine::Engine::bundled()?,
        &session.handshake().configuration,
    )?;
    session.run(plugin).await?;
    Ok(())
}
#[cfg(not(feature = "bundled-engine"))]
fn main() {
    eprintln!(
        "发布构建需要 bundled-engine 特性与 MEOW_ENGINE_BINARY；测试使用显式 Engine::command"
    );
    std::process::exit(64);
}
