//! 比较完整检测报告并通过宿主管理的 HTTP 发送飞书告警
use crate::{
    Service,
    domain::{Run, Status},
    host, invalid, management, now_ms,
};
use gateway_plugin_sdk::{PluginFault, call::host::HttpRequest, client::HostClient};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

pub(crate) struct Settings {
    webhook: String,
}
impl Settings {
    pub fn from_configuration(config: &Value) -> Self {
        Self {
            webhook: config["feishuWebhook"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        }
    }
    pub fn configured(&self) -> bool {
        [
            "https://open.feishu.cn/open-apis/bot/v2/hook/",
            "https://open.larksuite.com/open-apis/bot/v2/hook/",
        ]
        .iter()
        .any(|prefix| {
            self.webhook.strip_prefix(prefix).is_some_and(|token| {
                !token.is_empty()
                    && token
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        })
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Delivery {
    pub status: DeliveryStatus,
    pub attempts: u8,
    pub next_attempt_at: u64,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    #[default]
    Pending,
    Sending,
    Sent,
    NotRequired,
    Skipped,
    Failed,
}
impl Delivery {
    pub fn done(&self) -> bool {
        matches!(
            self.status,
            DeliveryStatus::Sent
                | DeliveryStatus::NotRequired
                | DeliveryStatus::Skipped
                | DeliveryStatus::Failed
        )
    }
}

pub(crate) fn mismatch(report: &Value, expected: &str) -> Option<String> {
    match report["method"].as_str().unwrap_or("meow") {
        "modeltrace"
            if report["verdict"] == "attributed"
                && report["valid_samples"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && Some(n) == report["planned_samples"].as_u64()) =>
        {
            let result = report["analysis"]["results"].as_array()?.first()?["model"].as_str()?;
            (!result.is_empty() && result != expected).then(|| result.to_owned())
        }
        "meow" if report["verdict"] == "mismatch" => {
            Some(report["model"].as_str().unwrap_or("行为偏移").to_owned())
        }
        _ => None,
    }
}

fn message(run: &Run, name: &str, expected: &str, result: &str) -> Value {
    json!({"msg_type":"text","content":{"text":format!(
        "模型检测告警\n定时任务：{name}\n账号：{}\n测试模型：{}\n预期模型：{expected}\n结果模型：{result}\n检测方法：{}\n检测记录：{}\n行为指纹与预期不匹配，请查看插件报告确认",
        run.account_name, run.plan["request_model"].as_str().unwrap_or_default(),
        run.plan["method"].as_str().unwrap_or("meow"), run.id)}})
}

async fn send(host: &HostClient, settings: &Settings, payload: Value) -> Result<(), PluginFault> {
    if !settings.configured() {
        return Err(invalid("飞书 Webhook 未配置或格式无效"));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let request = host.http(
        HttpRequest {
            method: "POST".into(),
            url: settings.webhook.clone(),
            headers: vec![("Content-Type".into(), b"application/json".to_vec())],
        },
        serde_json::to_vec(&payload).map_err(|_| invalid("飞书消息编码失败"))?,
    );
    let mut response = tokio::time::timeout_at(deadline, request)
        .await
        .map_err(|_| invalid("飞书发送超时，投递结果未确认"))?
        .map_err(|_| invalid("飞书发送失败，投递结果未确认"))?;
    // 保留正文句柄，以便超时或有界读取失败后立即关闭，而非等维护父调用结束
    let result = tokio::time::timeout_at(deadline, async {
        if !(200..300).contains(&response.status) {
            return Err(invalid(format!("飞书发送失败（HTTP {}）", response.status)));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .body
            .read()
            .await
            .map_err(|_| invalid("飞书响应读取失败，投递结果未确认"))?
        {
            if chunk.len() > (16 * 1024usize).saturating_sub(body.len()) {
                return Err(invalid("飞书响应超过大小限制，投递结果未确认"));
            }
            body.extend(chunk);
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|_| invalid("飞书响应格式错误，投递结果未确认"))?;
        if value["code"]
            .as_i64()
            .or_else(|| value["StatusCode"].as_i64())
            != Some(0)
        {
            return Err(invalid(
                "飞书拒绝消息，请检查机器人的关键词、IP 白名单和签名设置",
            ));
        }
        Ok(())
    })
    .await
    .unwrap_or_else(|_| Err(invalid("飞书响应超时，投递结果未确认")));
    let _ = tokio::time::timeout(Duration::from_secs(3), response.body.close()).await;
    result
}

pub(crate) async fn deliver_pending(
    service: &Service,
    client: &HostClient,
) -> Result<(), PluginFault> {
    let claimed = {
        let _guard = service.state.lock().await;
        let history = management::histories(client).await?;
        let Some((mut run, version)) = history.into_iter().find(|(r, _)| {
            r.finished()
                && r.scheduled.as_ref().is_some_and(|s| {
                    !s.alert.done()
                        && (s.alert.next_attempt_at <= now_ms() || !service.alerts.configured())
                })
        }) else {
            return Ok(());
        };
        let scheduled = run
            .scheduled
            .as_mut()
            .ok_or_else(|| invalid("缺少定时检测信息"))?;
        let result = if run.status == Status::Completed {
            mismatch(&run.report, &scheduled.expected_model)
        } else {
            None
        };
        if result.is_none() {
            scheduled.alert.status = DeliveryStatus::NotRequired;
            host::write(client, &run, Some(version)).await?;
            return Ok(());
        }
        // 告警是可选能力，缺少有效配置时终结投递，保留报告且不占用重试预算
        if !service.alerts.configured() {
            scheduled.alert.status = DeliveryStatus::Skipped;
            scheduled.alert.error = None;
            host::write(client, &run, Some(version)).await?;
            return Ok(());
        }
        if scheduled.alert.attempts >= 3 {
            scheduled.alert.status = DeliveryStatus::Failed;
            scheduled
                .alert
                .error
                .get_or_insert_with(|| "告警投递结果未确认，已达到 3 次尝试上限".into());
            host::write(client, &run, Some(version)).await?;
            return Ok(());
        }
        scheduled.alert.attempts += 1;
        scheduled.alert.status = DeliveryStatus::Sending;
        scheduled.alert.next_attempt_at = now_ms() + 60_000;
        let scheduled = run
            .scheduled
            .as_ref()
            .ok_or_else(|| invalid("缺少定时检测信息"))?;
        let payload = message(
            &run,
            &scheduled.name,
            &scheduled.expected_model,
            result.as_deref().unwrap_or_default(),
        );
        // 发送前保存尝试次数，宿主重复维护和进程重启不能绕过重试上限
        host::write(client, &run, Some(version)).await?;
        (run.id, payload)
    };
    let result = send(client, &service.alerts, claimed.1).await;
    let _guard = service.state.lock().await;
    let (mut run, version) = management::find(client, &claimed.0).await?;
    let delivery = &mut run
        .scheduled
        .as_mut()
        .ok_or_else(|| invalid("缺少定时检测信息"))?
        .alert;
    match result {
        Ok(()) => {
            delivery.status = DeliveryStatus::Sent;
            delivery.error = None;
        }
        Err(error) => {
            delivery.status = if delivery.attempts >= 3 {
                DeliveryStatus::Failed
            } else {
                DeliveryStatus::Pending
            };
            delivery.error = Some(error.message);
        }
    }
    host::write(client, &run, Some(version)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_complete_attribution_can_alert() {
        let mut report = json!({"method":"modeltrace","verdict":"attributed","valid_samples":3,"planned_samples":3,"analysis":{"results":[{"model":"gpt-6-sol"}]}});
        assert_eq!(
            mismatch(&report, "gpt-6-astra").as_deref(),
            Some("gpt-6-sol")
        );
        assert!(mismatch(&report, "gpt-6-sol").is_none());
        report["valid_samples"] = json!(2);
        assert!(mismatch(&report, "gpt-6-astra").is_none());
        report["verdict"] = json!("insufficient");
        assert!(mismatch(&report, "gpt-6-astra").is_none());
        assert!(mismatch(&json!({"verdict":"match"}), "x").is_none());
        assert_eq!(
            mismatch(&json!({"verdict":"mismatch","model":"other"}), "x").as_deref(),
            Some("other")
        );
    }
    #[test]
    fn webhook_requires_a_robot_endpoint() {
        for url in [
            "",
            "http://open.feishu.cn/open-apis/bot/v2/hook/a",
            "https://open.feishu.cn.evil.test/open-apis/bot/v2/hook/a",
            "https://open.feishu.cn/open-apis/bot/v2/hook/a?token=x",
        ] {
            assert!(!Settings::from_configuration(&json!({"feishuWebhook":url})).configured());
        }
        assert!(
            Settings::from_configuration(
                &json!({"feishuWebhook":"https://open.feishu.cn/open-apis/bot/v2/hook/test-id"})
            )
            .configured()
        );
    }
}
