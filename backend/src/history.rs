//! 已结束报告的持久归档；内存仅缓存索引，分页按需读取完整记录。
use crate::{Service, domain::Run, host, invalid, management};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use gateway_plugin_sdk::{
    PluginFault,
    call::host::{StateGetRequest, StateGetResult, StatePutRequest, StatePutResult},
    client::HostClient,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{Read, Write};

pub const LIMIT: usize = 1008;
const SHARD_SIZE: usize = 72;

fn encode(run: &Run) -> Result<Value, PluginFault> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    serde_json::to_writer(&mut encoder, run).map_err(|_| invalid("历史报告编码失败"))?;
    encoder.flush().map_err(|_| invalid("历史报告压缩失败"))?;
    let data = encoder.finish().map_err(|_| invalid("历史报告压缩失败"))?;
    let value = json!({"data":STANDARD.encode(data)});
    if serde_json::to_vec(&value)
        .map_err(|_| invalid("历史报告编码失败"))?
        .len()
        > 224 * 1024
    {
        return Err(invalid("历史报告压缩后超过存储上限，原检测记录已保留"));
    }
    Ok(value)
}
fn decode(value: Value) -> Result<Run, PluginFault> {
    let data = STANDARD
        .decode(
            value["data"]
                .as_str()
                .ok_or_else(|| invalid("历史报告格式错误"))?,
        )
        .map_err(|_| invalid("历史报告编码错误"))?;
    let mut bytes = vec![];
    GzDecoder::new(data.as_slice())
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("历史报告解压失败"))?;
    if bytes.len() > 256 * 1024 {
        return Err(invalid("历史报告解压超出限制"));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("历史报告格式错误"))
}

#[derive(Clone)]
struct Entry {
    id: String,
    created_at: u64,
    schedule_id: Option<String>,
    version: u64,
}
impl Entry {
    fn new(run: &Run, version: u64) -> Self {
        Self {
            id: run.id.clone(),
            created_at: run.created_at,
            schedule_id: run.scheduled.as_ref().map(|s| s.schedule_id.clone()),
            version,
        }
    }
}
#[derive(Default)]
pub(crate) struct Archive {
    entries: Option<Vec<Option<Entry>>>,
}

fn namespace(slot: usize) -> String {
    format!("history-{}", slot / SHARD_SIZE)
}
async fn read(host: &HostClient, slot: usize) -> Result<Option<(Run, u64)>, PluginFault> {
    let result: StateGetResult = host::metadata(
        host,
        "host.state.get",
        StateGetRequest {
            namespace: namespace(slot),
            key: format!("record-{slot}"),
        },
    )
    .await?;
    result
        .record
        .map(|r| {
            decode(r.value)
                .map(|run| (run, r.version))
                .map_err(|_| invalid("历史报告格式错误"))
        })
        .transpose()
}
impl Archive {
    async fn load(&mut self, host: &HostClient) -> Result<(), PluginFault> {
        if self.entries.is_some() {
            return Ok(());
        }
        let mut entries = Vec::with_capacity(LIMIT);
        for slot in 0..LIMIT {
            entries.push(
                read(host, slot)
                    .await?
                    .map(|(run, version)| Entry::new(&run, version)),
            );
        }
        self.entries = Some(entries);
        Ok(())
    }
}

pub(crate) async fn store(
    service: &Service,
    host: &HostClient,
    run: &Run,
) -> Result<(), PluginFault> {
    if !run.replaceable() {
        return Err(invalid("未结束或未完成告警的记录不能归档"));
    }
    let mut archive = service.history.lock().await;
    archive.load(host).await?;
    let entries = archive.entries.as_ref().unwrap();
    if entries.iter().flatten().any(|entry| entry.id == run.id) {
        return Ok(());
    }
    let slot = entries.iter().position(Option::is_none).unwrap_or_else(|| {
        entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| {
                let entry = entry.as_ref().unwrap();
                (entry.created_at, entry.id.as_str())
            })
            .unwrap()
            .0
    });
    let result: Result<StatePutResult, _> = host::metadata(
        host,
        "host.state.put",
        StatePutRequest {
            namespace: namespace(slot),
            key: format!("record-{slot}"),
            value: encode(run)?,
            expected_version: entries[slot].as_ref().map(|entry| entry.version),
        },
    )
    .await;
    match result {
        Ok(result) => {
            archive.entries.as_mut().unwrap()[slot] = Some(Entry::new(run, result.version));
            Ok(())
        }
        Err(error) => {
            archive.entries = None;
            Err(error)
        }
    }
}

pub(crate) async fn find(
    service: &Service,
    host: &HostClient,
    id: &str,
) -> Result<Option<Run>, PluginFault> {
    let mut archive = service.history.lock().await;
    archive.load(host).await?;
    let slot = archive
        .entries
        .as_ref()
        .unwrap()
        .iter()
        .position(|entry| entry.as_ref().is_some_and(|entry| entry.id == id));
    let Some(slot) = slot else { return Ok(None) };
    match read(host, slot).await {
        Ok(Some((run, _))) if run.id == id => Ok(Some(run)),
        _ => {
            archive.entries = None;
            Err(invalid("历史记录已变化，请刷新后重试"))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Query {
    pub page: usize,
    pub page_size: usize,
    pub schedule_id: Option<String>,
}

pub(crate) async fn list(
    service: &Service,
    host: &HostClient,
    query: Query,
) -> Result<Value, PluginFault> {
    if query.page == 0
        || ![20, 50, 100].contains(&query.page_size)
        || query
            .schedule_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 100)
    {
        return Err(invalid(
            "分页参数无效：页码从 1 开始，每页支持 20、50 或 100 条",
        ));
    }
    // 与执行槽位回收使用相同锁序；归档成功后才允许替换原记录。
    let _state = service.state.lock().await;
    let active = management::histories(host).await?;
    let mut archive = service.history.lock().await;
    archive.load(host).await?;
    let active_ids: BTreeSet<_> = active.iter().map(|(run, _)| run.id.as_str()).collect();
    let matches =
        |entry: &Entry| query.schedule_id.is_none() || entry.schedule_id == query.schedule_id;
    let mut entries: Vec<_> = archive
        .entries
        .as_ref()
        .unwrap()
        .iter()
        .enumerate()
        .filter_map(|(slot, entry)| {
            entry
                .as_ref()
                .filter(|entry| !active_ids.contains(entry.id.as_str()) && matches(entry))
                .map(|entry| (entry.clone(), Some(slot)))
        })
        .collect();
    entries.extend(
        active
            .iter()
            .map(|(run, version)| Entry::new(run, *version))
            .filter(matches)
            .map(|entry| (entry, None)),
    );
    entries.sort_by(|(a, _), (b, _)| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    let total = entries.len();
    let page = query.page.min(total.div_ceil(query.page_size).max(1));
    let mut runs = Vec::new();
    for (entry, slot) in entries
        .into_iter()
        .skip((page - 1) * query.page_size)
        .take(query.page_size)
    {
        let run = if let Some(slot) = slot {
            match read(host, slot).await {
                Ok(Some((run, _))) if run.id == entry.id => run,
                _ => {
                    archive.entries = None;
                    return Err(invalid("历史记录已变化，请刷新后重试"));
                }
            }
        } else {
            active
                .iter()
                .find(|(run, _)| run.id == entry.id)
                .unwrap()
                .0
                .clone()
        };
        runs.push(run.summary());
    }
    Ok(json!({"runs":runs,"total":total,"page":page,"page_size":query.page_size}))
}
