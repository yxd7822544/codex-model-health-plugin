//! 定时检测不符合预期时移入指定分组，并更新该计划的后续账号名单。
use crate::{
    Service, account_groups, alerts,
    domain::{Run, Status as RunStatus},
    host, invalid, management, now_ms, schedules,
};
use gateway_plugin_sdk::{PluginFault, call::host::HttpRequest, client::HostClient};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};
use tokio::sync::Mutex;

pub const GROUP_NAME: &str = "GPT降智分组";

pub(crate) struct Settings {
    api_key: String,
    execution: Mutex<()>,
}
impl Settings {
    pub fn new(config: &Value) -> Self {
        Self {
            api_key: config["hostAdminApiKey"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_owned(),
            execution: Mutex::new(()),
        }
    }
    pub fn configured(&self) -> bool {
        !self.api_key.is_empty()
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Action {
    pub status: Status,
    pub attempts: u8,
    pub next_attempt_at: u64,
    pub error: Option<String>,
    pub group_id: Option<String>,
    pub completed_at: Option<u64>,
    pub original_group_ids: Option<Vec<String>>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Pending,
    Moving,
    Succeeded,
    NotRequired,
    Failed,
}
impl Action {
    pub fn done(&self) -> bool {
        matches!(
            self.status,
            Status::Succeeded | Status::NotRequired | Status::Failed
        )
    }
}

async fn api(
    service: &Service,
    client: &HostClient,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, PluginFault> {
    let base = account_groups::origin(&service.host_api_base_url)?;
    let mut headers = vec![
        (
            "x-api-key".into(),
            service.remediation.api_key.as_bytes().to_vec(),
        ),
        ("Accept".into(), b"application/json".to_vec()),
    ];
    let body = if let Some(body) = body {
        headers.push(("Content-Type".into(), b"application/json".to_vec()));
        serde_json::to_vec(&body).map_err(|_| invalid("改组请求编码失败"))?
    } else {
        vec![]
    };
    let mut response = client
        .http(
            HttpRequest {
                method: method.into(),
                url: format!("{base}{path}"),
                headers,
            },
            body,
        )
        .await
        .map_err(|_| invalid("宿主改组接口请求失败"))?;
    if response.status != 200 {
        let status = response.status;
        let _ = response.body.close().await;
        return Err(invalid(format!(
            "宿主改组接口失败（HTTP {status}），请检查管理 API Key 和分组状态"
        )));
    }
    let bytes = response
        .body
        .collect(1024 * 1024)
        .await
        .map_err(|_| invalid("宿主改组响应读取失败"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid("宿主改组响应格式错误"))?;
    if value["code"] != 200 {
        return Err(invalid("宿主拒绝改组请求，请检查账号与分组状态"));
    }
    Ok(value["data"].clone())
}

async fn move_to_group(
    service: &Service,
    client: &HostClient,
    run: &mut Run,
) -> Result<String, PluginFault> {
    if !service.remediation.configured() {
        return Err(invalid("未配置管理 API Key，无法自动移入 GPT降智分组"));
    }
    host::validate_account(client, run).await?;
    if run.account_id.is_empty()
        || !run
            .account_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid("检测账号 ID 无效"));
    }
    let mut matches = vec![];
    for page in 1..=100 {
        let data = api(
            service,
            client,
            "GET",
            &format!("/api/admin/account-groups/options?page={page}&pageSize=100"),
            None,
        )
        .await?;
        let pages = data["page"]["totalPages"]
            .as_u64()
            .ok_or_else(|| invalid("分组目录分页格式错误"))?;
        if data["page"]["page"] != page || pages > 100 {
            return Err(invalid("分组目录分页异常"));
        }
        for group in data["items"]
            .as_array()
            .ok_or_else(|| invalid("分组目录格式错误"))?
        {
            if group["name"] == GROUP_NAME {
                matches.push(
                    group["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| invalid("目标分组 ID 无效"))?
                        .to_owned(),
                );
            }
        }
        if page >= pages {
            break;
        }
    }
    if matches.len() != 1 {
        return Err(invalid("请确保存在唯一的「GPT降智分组」，当前未改动账号"));
    }
    let target = matches.remove(0);
    let path = format!("/api/admin/accounts/detail?accountId={}", run.account_id);
    let data = api(service, client, "GET", &path, None).await?;
    let account = &data["account"];
    if account["id"] != run.account_id || account["provider"] != "openai" {
        return Err(invalid("宿主返回了不匹配的检测账号"));
    }
    let groups: BTreeSet<String> = account["groups"]
        .as_array()
        .ok_or_else(|| invalid("账号分组信息格式错误"))?
        .iter()
        .map(|group| {
            group["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid("账号分组 ID 无效"))
        })
        .collect::<Result<_, _>>()?;
    {
        let _state = service.state.lock().await;
        let (mut current, version) = management::find(client, &run.id).await?;
        let action = current
            .scheduled
            .as_mut()
            .unwrap()
            .remediation
            .as_mut()
            .unwrap();
        if action.original_group_ids.is_none() {
            action.original_group_ids = Some(groups.iter().cloned().collect());
            host::write(client, &current, Some(version)).await?;
        }
    }
    // 只保留目标组；重试时已经完成移组则只修正计划，不再次修改账号。
    if groups != [target.clone()].into() {
        api(
            service,
            client,
            "POST",
            "/api/admin/accounts/batch-update",
            Some(json!({"accountIds":[run.account_id],"groupIds":[target]})),
        )
        .await?;
        let verify = api(service, client, "GET", &path, None).await?;
        if verify["account"]["id"] != run.account_id
            || !verify["account"]["groups"]
                .as_array()
                .is_some_and(|actual| actual.len() == 1 && actual[0]["id"] == target)
        {
            return Err(invalid("账号改组结果尚未确认，后续会重试"));
        }
    }
    Ok(target)
}

pub(crate) async fn apply_pending(
    service: &Service,
    client: &HostClient,
) -> Result<(), PluginFault> {
    let _execution = service.remediation.execution.lock().await;
    let claimed = {
        let _state = service.state.lock().await;
        let Some((mut run, version)) =
            management::histories(client)
                .await?
                .into_iter()
                .find(|(run, _)| {
                    run.finished()
                        && run
                            .scheduled
                            .as_ref()
                            .and_then(|scheduled| scheduled.remediation.as_ref())
                            .is_some_and(|action| {
                                !action.done() && action.next_attempt_at <= now_ms()
                            })
                })
        else {
            return Ok(());
        };
        let invalid_report=run.data_error().is_some();
        let scheduled = run.scheduled.as_mut().unwrap();
        let action = scheduled.remediation.as_mut().unwrap();
        if invalid_report || run.status != RunStatus::Completed
            || run.task_error.is_some()
            || alerts::mismatch(&run.report, &scheduled.expected_model).is_none()
        {
            action.status = Status::NotRequired;
            host::write(client, &run, Some(version)).await?;
            return Ok(());
        }
        if action.attempts >= 3 {
            action.status = Status::Failed;
            action
                .error
                .get_or_insert_with(|| "自动改组已达到 3 次尝试上限，请检查配置后重新检测".into());
            host::write(client, &run, Some(version)).await?;
            return Ok(());
        }
        action.status = Status::Moving;
        action.attempts += 1;
        action.next_attempt_at = now_ms() + 60_000;
        host::write(client, &run, Some(version)).await?;
        run
    };
    let mut snapshot = claimed;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        move_to_group(service, client, &mut snapshot),
    )
    .await
    .unwrap_or_else(|_| Err(invalid("自动改组超时，结果尚未确认")));
    let result = match result {
        Ok(group) => schedules::remove_account(
            service,
            client,
            &snapshot.scheduled.as_ref().unwrap().schedule_id,
            &snapshot.account_id,
        )
        .await
        .map(|_| group),
        Err(error) => Err(error),
    };
    let _state = service.state.lock().await;
    let (mut run, version) = management::find(client, &snapshot.id).await?;
    let action = run
        .scheduled
        .as_mut()
        .unwrap()
        .remediation
        .as_mut()
        .unwrap();
    match result {
        Ok(group) => {
            action.status = Status::Succeeded;
            action.group_id = Some(group);
            action.completed_at = Some(now_ms());
            action.error = None;
        }
        Err(error) => {
            action.status = if action.attempts >= 3 || !service.remediation.configured() {
                Status::Failed
            } else {
                Status::Pending
            };
            action.error = Some(management::excerpt(&error.message));
        }
    }
    host::write(client, &run, Some(version)).await
}
