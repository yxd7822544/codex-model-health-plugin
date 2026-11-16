//! 定时检测配置、启停及到期入队，复用逐题执行与持久化合同
use crate::{
    Service,
    alerts::{Delivery, DeliveryStatus},
    domain::{Method, Run, Status},
    host, invalid,
    management::{self, Start},
    now_ms, now_nanos,
};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::host::{StateGetRequest, StateGetResult, StatePutRequest, StatePutResult},
    client::HostClient,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Serialize, Deserialize)]
pub struct ScheduledRun {
    pub schedule_id: String,
    pub name: String,
    pub due_at: u64,
    pub expected_model: String,
    pub alert: Delivery,
    // 轮次进度落盘前保留记录，避免槽位回收后重复创建或提前计时
    #[serde(default)]
    pub awaiting_checkpoint: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Member {
    account_id: String,
    run_id: Option<String>,
    finished_at: Option<u64>,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Round {
    due_at: u64,
    members: Vec<Member>,
    completed_at: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Schedule {
    id: String,
    name: String,
    enabled: bool,
    interval_minutes: u64,
    expected_model: String,
    detection: Start,
    #[serde(default)]
    account_ids: Vec<String>,
    #[serde(default)]
    round: Option<Round>,
    next_run_at: Option<u64>,
    last_run_id: Option<String>,
    error: Option<String>,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Save {
    id: Option<String>,
    revision: Option<u64>,
    name: String,
    interval_minutes: u64,
    expected_model: String,
    detection: Start,
    account_ids: Option<Vec<String>>,
    #[serde(default)]
    enable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Control {
    id: String,
    revision: u64,
    action: String,
}

async fn read(host: &HostClient) -> Result<(Vec<Schedule>, Option<u64>), PluginFault> {
    let result: StateGetResult = host::metadata(
        host,
        "host.state.get",
        StateGetRequest {
            namespace: "schedules".into(),
            key: "items".into(),
        },
    )
    .await?;
    match result.record {
        Some(record) => {
            let mut items: Vec<Schedule> = serde_json::from_value(record.value)
                .map_err(|_| invalid("定时任务状态格式错误"))?;
            // 已部署的单账号计划继续使用原账号，不改写其检测历史
            for item in &mut items {
                if item.account_ids.is_empty() {
                    item.account_ids.push(item.detection.account_id.clone());
                }
            }
            Ok((items, Some(record.version)))
        }
        None => Ok((vec![], None)),
    }
}

async fn write(
    host: &HostClient,
    items: &[Schedule],
    version: Option<u64>,
) -> Result<u64, PluginFault> {
    let result: StatePutResult = host::metadata(
        host,
        "host.state.put",
        StatePutRequest {
            namespace: "schedules".into(),
            key: "items".into(),
            value: serde_json::to_value(items).map_err(|_| invalid("定时任务编码失败"))?,
            expected_version: version,
        },
    )
    .await?;
    Ok(result.version)
}

pub(crate) async fn list(service: &Service, host: &HostClient) -> Result<Value, PluginFault> {
    let (items, _) = read(host).await?;
    let history = management::histories(host).await?;
    let items: Vec<_> = items.into_iter().map(|item| {
        let latest = history.iter().find(|(r, _)| Some(&r.id) == item.last_run_id.as_ref());
        let mut value = json!(item);
        value["last_run"] = latest.map_or(Value::Null, |(run, _)| json!({"id":run.id,"status":run.status,"inflight":run.inflight,"report":run.report,"alert":run.scheduled.as_ref().map(|s| &s.alert),"task_error":run.task_error}));
        if let Some(round) = &item.round {
            for (member, value) in round.members.iter().zip(value["round"]["members"].as_array_mut().unwrap()) {
                value["run"] = history.iter().find(|(r, _)| Some(&r.id) == member.run_id.as_ref())
                    .map_or(Value::Null, |(run, _)| {
                        let mut value = run.summary();
                        value["inflight"] = json!(run.inflight);
                        value["alert"] = json!(run.scheduled.as_ref().map(|s| &s.alert));
                        value
                    });
            }
        }
        value
    }).collect();
    Ok(json!({"schedules":items,"alerts_configured":service.alerts.configured()}))
}

pub(crate) async fn save(
    service: &Service,
    host: &HostClient,
    mut input: Save,
) -> Result<Value, PluginFault> {
    input.name = input.name.trim().to_owned();
    if input.name.is_empty()
        || input.name.chars().count() > 80
        || !(1..=43_200).contains(&input.interval_minutes)
    {
        return Err(invalid("请输入任务名称和 1～43200 分钟的间隔"));
    }
    input.detection.background = true;
    let account_ids = input
        .account_ids
        .unwrap_or_else(|| vec![input.detection.account_id.clone()]);
    let unique: std::collections::HashSet<_> = account_ids.iter().collect();
    if account_ids.is_empty() || unique.len() != account_ids.len() {
        return Err(invalid("请至少选择一个不重复的检测账号"));
    }
    input.detection.account_id = account_ids[0].clone();
    let catalog = service.engine.query(json!({"op":"catalog"})).await?;
    let method =
        serde_json::to_value(input.detection.method).map_err(|_| invalid("检测方法编码失败"))?;
    let valid_model = catalog["methods"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["method"] == method)
        .and_then(|m| m["models"].as_array())
        .is_some_and(|models| models.iter().any(|m| m["id"] == input.expected_model));
    if !valid_model {
        return Err(invalid("请选择检测候选库中的预期模型"));
    }
    if input.detection.method == Method::Meow
        && input.detection.claimed_model != input.expected_model
    {
        return Err(invalid("Meow 的预期模型必须与对照基线一致"));
    }
    let accounts = host::accounts(host).await?;
    if !account_ids
        .iter()
        .all(|id| accounts.iter().any(|a| &a.account_id == id))
        || !host::keys(host)
            .await?
            .iter()
            .any(|k| k.id == input.detection.key_id)
    {
        return Err(invalid("请选择已启用的 OpenAI 账号和测试 Key"));
    }
    management::prepare(service, host, &input.detection).await?;
    let _guard = service.schedules.lock().await;
    let (mut items, version) = read(host).await?;
    if let Some(id) = input.id {
        let item = items
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| invalid("定时任务不存在"))?;
        if input.revision != Some(item.revision) {
            return Err(invalid("定时任务已更新，请刷新后编辑"));
        }
        if item.enabled {
            return Err(invalid("请先停用定时任务再编辑"));
        }
        if unfinished(item, &management::histories(host).await?) {
            return Err(invalid("本轮尚未结束，请先处理本轮检测"));
        }
        item.name = input.name;
        item.interval_minutes = input.interval_minutes;
        item.expected_model = input.expected_model;
        item.detection = input.detection;
        item.account_ids = account_ids;
        item.enabled = input.enable;
        item.next_run_at = input.enable.then(now_ms);
        item.error = None;
        item.revision += 1;
    } else {
        if items.len() >= 16 {
            return Err(invalid("最多保存 16 条定时任务"));
        }
        items.push(Schedule {
            id: now_nanos().to_string(),
            name: input.name,
            enabled: input.enable,
            interval_minutes: input.interval_minutes,
            expected_model: input.expected_model,
            detection: input.detection,
            account_ids,
            round: None,
            next_run_at: input.enable.then(now_ms),
            last_run_id: None,
            error: None,
            revision: 1,
        });
    }
    write(host, &items, version).await?;
    if input.enable {
        service.wake.notify_one();
    }
    Ok(json!({"saved":true}))
}

pub(crate) async fn control(
    service: &Service,
    host: &HostClient,
    input: Control,
) -> Result<Value, PluginFault> {
    let _guard = service.schedules.lock().await;
    let (mut items, version) = read(host).await?;
    let item = items
        .iter_mut()
        .find(|s| s.id == input.id)
        .ok_or_else(|| invalid("定时任务不存在"))?;
    if item.revision != input.revision {
        return Err(invalid("定时任务已更新，请刷新后操作"));
    }
    match input.action.as_str() {
        "enable" => {
            if !item.enabled {
                item.enabled = true;
                item.next_run_at = if unfinished(item, &management::histories(host).await?) {
                    None
                } else {
                    Some(now_ms())
                };
                item.error = None;
            }
        }
        "disable" => {
            item.enabled = false;
            item.next_run_at = None;
        }
        "run_now" => {
            if !item.enabled {
                return Err(invalid("请先启用定时任务"));
            }
            let history = management::histories(host).await?;
            if unfinished(item, &history) {
                return Err(invalid("本轮检测尚未结束，请先查看检测任务"));
            }
            item.next_run_at = Some(now_ms());
        }
        "delete" => {
            if item.enabled {
                return Err(invalid("请先停用定时任务再删除"));
            }
            if unfinished(item, &management::histories(host).await?) {
                return Err(invalid("本轮尚未结束，请先处理本轮检测"));
            }
            items.retain(|s| s.id != input.id);
            write(host, &items, version).await?;
            return Ok(json!({"saved":true}));
        }
        _ => return Err(invalid("未知的定时任务操作")),
    }
    item.revision += 1;
    write(host, &items, version).await?;
    service.wake.notify_one();
    Ok(json!({"saved":true}))
}

pub(crate) async fn tick(service: &Service, host: &HostClient) -> Result<(), PluginFault> {
    let _guard = service.schedules.lock().await;
    let (mut items, mut version) = read(host).await?;
    finish_paused(service, host).await?;
    acknowledge(service, host, &items).await?;
    for index in 0..items.len() {
        let mut item = items[index].clone();
        let mut changed = false;
        let mut history = management::histories(host).await?;
        // 接管升级前尚未结束的单账号轮次，仍等待原检测完成
        if item.round.is_none()
            && let Some((run, _)) = history.iter().find(|(r, _)| {
                r.scheduled
                    .as_ref()
                    .is_some_and(|s| s.schedule_id == item.id)
                    && !r.finished()
            })
        {
            item.round = Some(Round {
                due_at: run.scheduled.as_ref().unwrap().due_at,
                members: vec![Member {
                    account_id: run.account_id.clone(),
                    run_id: Some(run.id.clone()),
                    finished_at: None,
                    error: None,
                }],
                completed_at: None,
            });
            item.next_run_at = None;
            changed = true;
        }
        if item.round.as_ref().is_none_or(|r| r.completed_at.is_some()) {
            let Some(due) = item
                .next_run_at
                .filter(|due| item.enabled && *due <= now_ms())
            else {
                continue;
            };
            item.round = Some(Round {
                due_at: due,
                members: item
                    .account_ids
                    .iter()
                    .map(|id| Member {
                        account_id: id.clone(),
                        run_id: None,
                        finished_at: None,
                        error: None,
                    })
                    .collect(),
                completed_at: None,
            });
            item.next_run_at = None;
            item.error = None;
            changed = true;
        }
        // 先固定本轮账号，再创建检测；停用或重启也能接着处理同一轮
        if changed {
            items[index] = item.clone();
            version = Some(write(host, &items, version).await?);
            changed = false;
        }
        let round = item.round.as_mut().unwrap();
        for member in &mut round.members {
            if member.finished_at.is_some() {
                continue;
            }
            // 入队写入成功、计划提交失败时，按轮次和账号找回原检测
            if let Some((run, _)) = history.iter().find(|(r, _)| {
                r.scheduled
                    .as_ref()
                    .is_some_and(|s| s.schedule_id == item.id && s.due_at == round.due_at)
                    && r.account_id == member.account_id
            }) {
                if member.run_id.as_ref() != Some(&run.id) {
                    member.run_id = Some(run.id.clone());
                    item.last_run_id = Some(run.id.clone());
                    changed = true;
                }
                if run.finished() {
                    member.finished_at = Some(run.updated_at);
                    member.error = run.task_error.clone();
                    changed = true;
                }
                continue;
            }
            if member.run_id.is_some() {
                return Err(invalid("本轮检测记录缺失，无法确认执行结果"));
            }
            let scheduled = ScheduledRun {
                schedule_id: item.id.clone(),
                name: item.name.clone(),
                due_at: round.due_at,
                expected_model: item.expected_model.clone(),
                alert: Delivery::default(),
                awaiting_checkpoint: true,
            };
            let mut detection = item.detection.clone();
            detection.account_id = member.account_id.clone();
            match management::create_run(service, host, detection, Some(scheduled)).await {
                Ok(run) => {
                    member.run_id = Some(run.id.clone());
                    item.last_run_id = Some(run.id.clone());
                    item.error = None;
                    history.push((run, 0));
                }
                Err(error) if error.code == ErrorCode::Capacity => {
                    let message = Some(management::excerpt(&error.message));
                    changed |= item.error != message;
                    item.error = message;
                    break;
                }
                Err(error) if error.code == ErrorCode::InvalidInput => {
                    member.error = Some(management::excerpt(&error.message));
                    member.finished_at = Some(now_ms());
                }
                Err(error) => return Err(error),
            }
            changed = true;
        }
        if round.members.iter().all(|m| m.finished_at.is_some()) {
            let completed = round
                .members
                .iter()
                .filter_map(|m| m.finished_at)
                .max()
                .unwrap_or_else(now_ms);
            round.completed_at = Some(completed);
            item.next_run_at = item
                .enabled
                .then_some(completed + item.interval_minutes * 60_000);
            item.error = None;
            changed = true;
        }
        if changed {
            items[index] = item;
            version = Some(write(host, &items, version).await?);
        }
    }
    acknowledge(service, host, &items).await?;
    Ok(())
}

async fn finish_paused(service: &Service, host: &HostClient) -> Result<(), PluginFault> {
    let _guard = service.state.lock().await;
    for (mut run, version) in management::histories(host).await? {
        if run.scheduled.is_none() || run.status != Status::Paused || run.inflight.is_some() {
            continue;
        }
        // 单个账号结束本轮尝试，保留错误；进度落盘前仍不能回收槽位。
        run.status = Status::Stopped;
        run.queued_at = None;
        run.updated_at = now_ms();
        run.task_error.get_or_insert_with(|| "检测已暂停，本轮已跳过该账号".into());
        let scheduled = run.scheduled.as_mut().unwrap();
        scheduled.awaiting_checkpoint = true;
        scheduled.alert.status = DeliveryStatus::NotRequired;
        host::write(host, &run, Some(version)).await?;
    }
    Ok(())
}

fn unfinished(item: &Schedule, history: &[(Run, u64)]) -> bool {
    item.round
        .as_ref()
        .is_some_and(|r| r.completed_at.is_none())
        || history.iter().any(|(r, _)| {
            r.scheduled
                .as_ref()
                .is_some_and(|s| s.schedule_id == item.id)
                && !r.finished()
        })
}

async fn acknowledge(
    service: &Service,
    host: &HostClient,
    items: &[Schedule],
) -> Result<(), PluginFault> {
    let _guard = service.state.lock().await;
    for (mut run, version) in management::histories(host).await? {
        let Some(scheduled) = run.scheduled.as_ref().filter(|s| s.awaiting_checkpoint) else {
            continue;
        };
        let checkpointed = items
            .iter()
            .find(|item| item.id == scheduled.schedule_id)
            .is_none_or(|item| {
                item.round.as_ref().is_some_and(|round| {
                    round.members.iter().any(|member| {
                        member.run_id.as_ref() == Some(&run.id) && member.finished_at.is_some()
                    })
                })
            });
        if checkpointed {
            run.scheduled.as_mut().unwrap().awaiting_checkpoint = false;
            host::write(host, &run, Some(version)).await?;
        }
    }
    Ok(())
}
