//! 用宿主分组目录和账号事实生成可勾选的分组，不保存管理员会话。
use crate::invalid;
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{data::AccountFactsQuery, host::HttpRequest, management::ManagementRequest},
    client::HostClient,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    time::Duration,
};

#[derive(Deserialize, Serialize)]
pub(crate) struct Group {
    id: String,
    name: String,
    enabled: bool,
    #[serde(default)]
    account_ids: Vec<String>,
}

#[derive(Deserialize)]
struct Envelope {
    code: u16,
    data: GroupPage,
}
#[derive(Deserialize)]
struct GroupPage {
    items: Vec<Group>,
    page: Page,
}
#[derive(Deserialize)]
struct Page {
    page: u32,
    #[serde(rename = "totalPages")]
    total_pages: u32,
}

fn origin(value: &str) -> Result<String, PluginFault> {
    let address: SocketAddr = value
        .trim()
        .trim_end_matches('/')
        .strip_prefix("http://")
        .and_then(|value| value.parse().ok())
        .filter(|address: &SocketAddr| address.ip().is_loopback() && address.port() != 0)
        .ok_or_else(|| {
            invalid("宿主管理地址须为 HTTP 回环 IP 和端口，例如 http://127.0.0.1:8080")
        })?;
    Ok(format!("http://{address}"))
}

fn session_cookie(request: &ManagementRequest) -> Result<Vec<u8>, PluginFault> {
    for header in &request.headers {
        if !header.name.eq_ignore_ascii_case("cookie") {
            continue;
        }
        let Ok(value) = std::str::from_utf8(&header.value) else {
            continue;
        };
        for pair in value.split(';') {
            if let Some(token) = pair.trim().strip_prefix("cpr_session=") {
                if !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()) {
                    return Ok(format!("cpr_session={token}").into_bytes());
                }
            }
        }
    }
    Err(invalid(
        "读取账号分组需要管理员登录会话，请重新登录宿主后打开插件",
    ))
}

pub(crate) async fn list(
    host: &HostClient,
    base: &str,
    request: &ManagementRequest,
) -> Result<Vec<Group>, PluginFault> {
    let base = origin(base)?;
    let cookie = session_cookie(request)?;
    tokio::time::timeout(Duration::from_secs(15), load(host, &base, cookie))
        .await
        .map_err(|_| PluginFault::new(ErrorCode::Timeout, "读取账号分组超时，请重试"))?
}

async fn load(host: &HostClient, base: &str, cookie: Vec<u8>) -> Result<Vec<Group>, PluginFault> {
    let mut groups = vec![];
    let mut indexes = BTreeMap::new();
    for page in 1..=100 {
        let mut response = host
            .http(
                HttpRequest {
                    method: "GET".into(),
                    url: format!(
                        "{base}/api/admin/account-groups/options?page={page}&pageSize=100"
                    ),
                    headers: vec![
                        ("Cookie".into(), cookie.clone()),
                        ("Accept".into(), b"application/json".to_vec()),
                    ],
                },
                vec![],
            )
            .await
            .map_err(|_| invalid("无法连接宿主分组目录，请检查插件设置中的宿主管理地址"))?;
        if response.status != 200 {
            let status = response.status;
            let _ = response.body.close().await;
            return Err(invalid(format!(
                "读取宿主分组目录失败（HTTP {status}），请检查管理员会话"
            )));
        }
        let bytes = response
            .body
            .collect(256 * 1024)
            .await
            .map_err(|_| invalid("宿主分组目录响应超出限制或读取失败"))?;
        let result: Envelope =
            serde_json::from_slice(&bytes).map_err(|_| invalid("宿主分组目录格式错误"))?;
        if result.code != 200 || result.data.page.page != page || result.data.page.total_pages > 100
        {
            return Err(invalid("宿主分组目录分页异常，请刷新后重试"));
        }
        for mut group in result.data.items {
            if group.id.is_empty() || indexes.insert(group.id.clone(), groups.len()).is_some() {
                return Err(invalid("宿主分组目录已变化，请刷新后重试"));
            }
            group.account_ids.clear();
            groups.push(group);
        }
        if page >= result.data.page.total_pages {
            break;
        }
    }
    let ungrouped = groups.len();
    groups.push(Group {
        id: String::new(),
        name: "未分组".into(),
        enabled: true,
        account_ids: vec![],
    });
    let mut cursor = None;
    let mut cursors = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for _ in 0..100 {
        let page = host
            .account_facts(AccountFactsQuery {
                provider_id: Some("openai".into()),
                cursor,
                limit: 100,
            })
            .await?;
        for account in page.accounts {
            if !account.enabled
                || account.provider_id != "openai"
                || !seen.insert(account.account_id.clone())
            {
                continue;
            }
            if account.group_ids.is_empty() {
                groups[ungrouped].account_ids.push(account.account_id);
            } else {
                for id in account.group_ids.into_iter().collect::<BTreeSet<_>>() {
                    let index = indexes
                        .get(&id)
                        .ok_or_else(|| invalid("账号分组已变化，请刷新后重试"))?;
                    groups[*index].account_ids.push(account.account_id.clone());
                }
            }
        }
        cursor = page.next_cursor;
        let Some(next) = &cursor else {
            return Ok(groups);
        };
        if !cursors.insert(next.clone()) {
            return Err(invalid("账号分组分页重复，请重试"));
        }
    }
    Err(invalid("账号分组分页超过 100 页"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn group_directory_only_uses_loopback_origins() {
        for value in [
            "https://example.com",
            "http://example.com:8080",
            "http://192.168.1.1:8080",
            "http://127.0.0.1:0",
            "http://127.0.0.1:8080/path",
            "http://user@127.0.0.1:8080",
        ] {
            assert!(origin(value).is_err(), "{value}");
        }
        assert_eq!(
            origin("http://127.0.0.1:8080/").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(origin("http://[::1]:8080").unwrap(), "http://[::1]:8080");
    }
    #[test]
    fn only_the_current_session_cookie_is_forwarded() {
        let mut request: ManagementRequest = serde_json::from_value(serde_json::json!({"method":"GET","path":"api/account-groups","query":"","content_type":null,"headers":[{"name":"Cookie","value":b"other=secret; cpr_session=fixture; cpr_refresh=private".to_vec()}]})).unwrap();
        assert_eq!(session_cookie(&request).unwrap(), b"cpr_session=fixture");
        request.headers.clear();
        assert!(session_cookie(&request).is_err());
    }
}
