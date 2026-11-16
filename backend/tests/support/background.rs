//! 维护、定时任务与管理请求并发下的生命周期和通知验证
use super::*;

#[derive(Default)]
struct Schedule {
    epoch: u64,
    suspended: bool,
    resident: BTreeSet<u64>,
}
struct Background {
    peer: Peer,
    id: u64,
    calls: BTreeMap<u64, Schedule>,
}

impl Background {
    async fn new(mut peer: Peer, timeout_ms: u64) -> Self {
        let id = peer.sequence;
        peer.sequence += 2;
        peer.send(Frame::control(Message::Call {
            id,
            method: "plugin.reconcile".into(),
            params: json!({}),
            context: CallContext {
                request: None,
                call_id: id,
                instance_id: "test".into(),
                generation: 1,
                incarnation: "test".into(),
                stage: Stage::Maintenance,
                timeout_ms,
                resource_stream: false,
                resource_scope_id: "maintenance".into(),
                request_id: None,
                attempt_id: None,
                account_id: None,
                credential_revision: None,
            },
        }))
        .await;
        Self {
            peer,
            id,
            calls: BTreeMap::new(),
        }
    }
    async fn tick(&mut self, host: &mut Host) -> Option<(u64, Result<Value, PluginFault>)> {
        let frame = self.peer.receive().await;
        match frame.message {
            Message::Callback {
                id,
                parent_id,
                method,
                params,
            } => {
                let s = self.calls.entry(parent_id).or_default();
                assert!(!s.suspended && s.resident.insert(id));
                if method.starts_with("host.model.") {
                    assert_eq!(parent_id, self.id);
                }
                let response = match host.callback(&method, params, frame.payload) {
                    Ok((result, payload)) => Frame {
                        message: Message::Result { id, result },
                        payload,
                    },
                    Err(error) => Frame::control(Message::Error { id, error }),
                };
                self.peer.send(response).await;
            }
            Message::Suspend { id, epoch } => {
                let s = self.calls.entry(id).or_default();
                assert!(!s.suspended);
                assert_eq!(epoch, s.epoch + 1);
                s.epoch = epoch;
                s.suspended = true;
            }
            Message::Resume { id, epoch } => {
                let s = self.calls.get_mut(&id).unwrap();
                assert!(s.suspended);
                assert_eq!(epoch, s.epoch);
                self.peer
                    .send(Frame::control(Message::Resumed { id, epoch }))
                    .await;
                s.suspended = false;
            }
            Message::CallbackConsumed { id, parent_id } => {
                let s = self.calls.get_mut(&parent_id).unwrap();
                assert!(!s.suspended && s.resident.remove(&id));
            }
            Message::Result { id, result } => {
                let s = self.calls.remove(&id).unwrap_or_default();
                assert!(!s.suspended && s.resident.is_empty());
                let body = if frame.payload.is_empty() {
                    result.clone()
                } else {
                    serde_json::from_slice(&frame.payload).unwrap()
                };
                if result["status"].as_u64().is_some_and(|s| s >= 400) {
                    return Some((
                        id,
                        Err(serde_json::from_value(body["error"].clone()).unwrap()),
                    ));
                }
                return Some((id, Ok(body)));
            }
            Message::Error { id, error } => return Some((id, Err(error))),
            Message::Cancelled { id } => return Some((id, Ok(Value::Null))),
            other => panic!("unexpected background frame {other:?}"),
        }
        None
    }
    async fn until(&mut self, host: &mut Host, done: impl Fn(&Host) -> bool) {
        while !done(host) {
            assert!(
                self.tick(host).await.is_none(),
                "maintenance ended unexpectedly"
            );
        }
    }
    async fn api(
        &mut self,
        path: &str,
        input: Option<Value>,
        host: &mut Host,
    ) -> Result<Value, PluginFault> {
        let id = self.peer.begin_api(path, input).await;
        loop {
            if let Some((finished, result)) = self.tick(host).await {
                assert_eq!(finished, id);
                return result;
            }
        }
    }
    async fn finish(&mut self, host: &mut Host) -> Result<Value, PluginFault> {
        loop {
            if let Some((id, result)) = self.tick(host).await {
                assert_eq!(id, self.id);
                return result;
            }
        }
    }
    async fn cancel(mut self, host: &mut Host) -> Peer {
        self.peer
            .send(Frame::control(Message::Cancel { id: self.id }))
            .await;
        self.finish(host).await.unwrap();
        // Real host cancellation reclaims this parent's streams.
        host.model_open = false;
        host.model_events = None;
        self.peer
    }
}
async fn queued(peer: &mut Peer, host: &mut Host) -> Value {
    peer.api("api/runs/start",Some(json!({"background":true,"account_id":"account-b","key_id":"key-b","request_model":"gpt-6-astra","claimed_model":"gpt-6-astra","tier":"low"})),host).await.unwrap()["run"].clone()
}
fn saved<'a>(host: &'a Host, id: &Value) -> &'a Value {
    &host.state.values().find(|(r, _)| r["id"] == *id).unwrap().0
}

#[tokio::test]
async fn finishes_without_page_steps_and_reopens_completed() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let run = queued(&mut peer, &mut host).await;
    assert!(host.executions.is_empty());
    assert!(
        peer.api("api/runs/step", Some(json!({"id":run["id"]})), &mut host)
            .await
            .unwrap_err()
            .message
            .contains("后台托管")
    );
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| saved(h, &run["id"])["status"] == "completed")
        .await;
    assert_eq!(host.executions.len(), 36);
    assert_eq!(saved(&host, &run["id"])["report"]["valid_samples"], 36);
    assert!(!host.model_open && !host.callbacks.iter().any(|m| m.starts_with("host.http.")));
    let reopened = bg
        .api("api/runs/get", Some(json!({"id":run["id"]})), &mut host)
        .await
        .unwrap();
    assert_eq!(reopened["run"]["status"], "completed");
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn pause_preserves_inflight_and_resume_queues_at_tail() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let first = queued(&mut peer, &mut host).await;
    let second = queued(&mut peer, &mut host).await;
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| h.executions.len() == 1).await;
    let again = bg
        .api(
            "api/runs/control",
            Some(json!({"id":first["id"],"action":"resume"})),
            &mut host,
        )
        .await
        .unwrap();
    assert_eq!(again["run"]["queued_at"], first["queued_at"]);
    bg.api(
        "api/runs/control",
        Some(json!({"id":first["id"],"action":"pause"})),
        &mut host,
    )
    .await
    .unwrap();
    bg.until(&mut host, |h| saved(h, &first["id"])["inflight"].is_null())
        .await;
    let before = saved(&host, &first["id"])["attempts"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(saved(&host, &first["id"])["status"], "paused");
    let resumed = bg
        .api(
            "api/runs/control",
            Some(json!({"id":first["id"],"action":"resume"})),
            &mut host,
        )
        .await
        .unwrap();
    assert!(resumed["run"]["queued_at"].as_u64() > second["queued_at"].as_u64());
    bg.until(&mut host, |h| {
        saved(h, &second["id"])["status"] == "completed"
    })
    .await;
    assert_eq!(
        saved(&host, &first["id"])["attempts"]
            .as_array()
            .unwrap()
            .len(),
        before
    );
    bg.api(
        "api/runs/control",
        Some(json!({"id":first["id"],"action":"stop"})),
        &mut host,
    )
    .await
    .unwrap();
    bg.until(&mut host, |h| saved(h, &first["id"])["inflight"].is_null())
        .await;
    assert_eq!(saved(&host, &first["id"])["status"], "stopped");
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn error_pauses_only_its_run_and_continues_next() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let first = queued(&mut peer, &mut host).await;
    let second = queued(&mut peer, &mut host).await;
    host.fail = true;
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| saved(h, &first["id"])["status"] == "paused")
        .await;
    assert_eq!(host.executions.len(), 1);
    assert!(
        saved(&host, &first["id"])["task_error"]
            .as_str()
            .unwrap()
            .contains("network")
    );
    assert!(
        !saved(&host, &first["id"])
            .to_string()
            .contains("fixture-secret")
    );
    host.fail = false;
    bg.until(&mut host, |h| {
        saved(h, &second["id"])["status"] == "completed"
    })
    .await;
    assert_eq!(host.executions.len(), 37);
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn host_execution_errors_pause_without_fallback_and_continue_the_queue() {
    use gateway_plugin_sdk::SendState;
    for (code, http_status, send_state) in [
        (ErrorCode::Capacity, None, SendState::NotSent),
        (ErrorCode::Timeout, None, SendState::NotSent),
        (ErrorCode::Upstream, Some(503), SendState::Sent),
        (ErrorCode::Upstream, Some(503), SendState::Ambiguous),
    ] {
        let mut peer = Peer::new().await;
        let mut host = Host::default();
        let first = queued(&mut peer, &mut host).await;
        let second = queued(&mut peer, &mut host).await;
        let mut error = PluginFault::new(code, "fixture host execution rejected");
        error.http_status = http_status;
        error.send_state = send_state;
        error.details = Some(json!({"diagnostic":"fixture-secret"}));
        host.execute_error = Some(error);
        let mut bg = Background::new(peer, 3_600_000).await;
        bg.until(&mut host, |h| saved(h, &first["id"])["status"] == "paused")
            .await;
        let rejected = saved(&host, &first["id"]);
        assert_eq!(rejected["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(rejected["attempts"][0]["error_code"], json!(code));
        assert_eq!(rejected["attempts"][0]["http_status"], json!(http_status));
        assert_eq!(rejected["attempts"][0]["send_state"], json!(send_state));
        assert!(rejected["attempts"][0]["request_id"].is_null());
        assert_eq!(rejected["report"]["valid_samples"], 0);
        assert_eq!(rejected["task_error"], "fixture host execution rejected");
        assert!(rejected["inflight"].is_null() && rejected["queued_at"].is_null());
        assert!(!rejected.to_string().contains("fixture-secret"));
        assert_eq!(host.executions.len(), 1);
        assert!(!host.model_open);
        assert!(!host.callbacks.iter().any(|m| m == "host.model.stream_read"));
        bg.until(&mut host, |h| {
            saved(h, &second["id"])["status"] == "completed"
        })
        .await;
        assert_eq!(host.executions.len(), 37);
        assert_eq!(saved(&host, &first["id"])["status"], "paused");
        assert_eq!(
            saved(&host, &first["id"])["attempts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(!host.callbacks.iter().any(|m| m.starts_with("host.http.")));
        bg.cancel(&mut host).await;
    }
}

#[tokio::test]
async fn account_change_pauses_before_sending() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let run = queued(&mut peer, &mut host).await;
    host.account_patch = Some(json!({"upstream_account_id":"replacement"}));
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| saved(h, &run["id"])["status"] == "paused")
        .await;
    assert!(host.executions.is_empty());
    assert!(
        saved(&host, &run["id"])["task_error"]
            .as_str()
            .unwrap()
            .contains("身份已改变")
    );
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn restart_pauses_all_and_charges_unknown_once() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let first = queued(&mut peer, &mut host).await;
    let second = queued(&mut peer, &mut host).await;
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| h.executions.len() == 1).await;
    drop(bg);
    host.model_open = false;
    host.model_events = None;
    let mut peer = Peer::new().await;
    peer.api("api/runs", None, &mut host).await.unwrap();
    for run in [&first, &second] {
        assert_eq!(saved(&host, &run["id"])["status"], "paused");
        assert!(saved(&host, &run["id"])["queued_at"].is_null());
    }
    assert_eq!(
        saved(&host, &first["id"])["attempts"][0]["error"],
        "interrupted_unknown_result"
    );
    assert_eq!(host.executions.len(), 1);
    let third = queued(&mut peer, &mut host).await;
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| {
        saved(h, &third["id"])["status"] == "completed"
    })
    .await;
    assert_eq!(
        saved(&host, &first["id"])["attempts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(host.executions.len(), 37);
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn persistence_failure_ends_parent_and_pauses_without_repeating() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let first = queued(&mut peer, &mut host).await;
    let second = queued(&mut peer, &mut host).await;
    host.reject_completed_write = true;
    let mut bg = Background::new(peer, 3_600_000).await;
    assert!(
        bg.finish(&mut host)
            .await
            .unwrap_err()
            .message
            .contains("state unavailable")
    );
    assert_eq!(host.executions.len(), 1);
    host.reject_completed_write = false;
    let mut bg = Background::new(bg.peer, 150_000).await;
    bg.finish(&mut host).await.unwrap();
    assert_eq!(saved(&host, &first["id"])["status"], "paused");
    assert_eq!(saved(&host, &second["id"])["status"], "paused");
    assert_eq!(host.executions.len(), 1);
}

#[tokio::test]
async fn handoff_requires_a_full_question_window() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let run = queued(&mut peer, &mut host).await;
    let mut bg = Background::new(peer, 150_000).await;
    bg.finish(&mut host).await.unwrap();
    assert_eq!(saved(&host, &run["id"])["status"], "ready");
    assert!(host.executions.is_empty());
    let mut bg = Background::new(bg.peer, 3_600_000).await;
    bg.until(&mut host, |h| saved(h, &run["id"])["status"] == "completed")
        .await;
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn preparation_and_scoring_errors_are_persisted_without_losing_samples() {
    for after_send in [false, true] {
        let mut peer = Peer::new().await;
        let mut host = Host::default();
        let run = queued(&mut peer, &mut host).await;
        let mut bg = Background::new(peer, 3_600_000).await;
        if after_send {
            bg.until(&mut host, |h| h.executions.len() == 1 && !h.model_open)
                .await;
        }
        host.state.values_mut().next().unwrap().0["plan"]["hash"] = json!("changed");
        bg.until(&mut host, |h| saved(h, &run["id"])["status"] == "paused")
            .await;
        let state = saved(&host, &run["id"]);
        assert!(state["inflight"].is_null());
        assert!(
            state["task_error"]
                .as_str()
                .unwrap()
                .contains("baseline_changed")
        );
        assert_eq!(host.executions.len(), usize::from(after_send));
        if after_send {
            assert_eq!(state["attempts"][0]["status"], "ok");
        }
        // Ending a task remains possible even when its scorer cannot load the plan.
        let stopped = bg
            .api(
                "api/runs/control",
                Some(json!({"id":run["id"],"action":"stop"})),
                &mut host,
            )
            .await
            .unwrap();
        assert_eq!(stopped["run"]["status"], "stopped");
        bg.cancel(&mut host).await;
    }
}

#[tokio::test]
async fn idle_maintenance_accepts_new_work_and_legacy_resume_can_opt_in() {
    let peer = Peer::new().await;
    let mut host = Host::default();
    let mut bg = Background::new(peer, 3_600_000).await;
    // Complete the initialization scan and let maintenance suspend waiting for work.
    bg.until(&mut host, |h| {
        h.callbacks
            .iter()
            .filter(|m| *m == "host.state.get")
            .count()
            >= 32
    })
    .await;
    let manual=bg.api("api/runs/start",Some(json!({"account_id":"account-b","key_id":"key-b","request_model":"gpt-6-astra","claimed_model":"gpt-6-astra","tier":"low"})),&mut host).await.unwrap()["run"].clone();
    assert_eq!(manual["background"], false);
    bg.api(
        "api/runs/control",
        Some(json!({"id":manual["id"],"action":"resume","background":true})),
        &mut host,
    )
    .await
    .unwrap();
    bg.until(&mut host, |h| {
        saved(h, &manual["id"])["status"] == "completed"
    })
    .await;
    assert_eq!(host.executions.len(), 36);
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn cancelled_maintenance_recovers_pending_in_same_process_without_resending() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    let run = queued(&mut peer, &mut host).await;
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| h.executions.len() == 1).await;
    let peer = bg.cancel(&mut host).await;
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| saved(h, &run["id"])["status"] == "paused")
        .await;
    assert_eq!(host.executions.len(), 1);
    assert_eq!(
        saved(&host, &run["id"])["attempts"][0]["error"],
        "interrupted_unknown_result"
    );
    bg.cancel(&mut host).await;
}

async fn scheduled(peer: &mut Peer, host: &mut Host) -> Value {
    peer.api("api/schedules/save", Some(json!({"name":"模型巡检","interval_minutes":60,"expected_model":"gpt-5.4","detection":{"method":"modeltrace","background":true,"account_id":"account-b","key_id":"key-b","request_model":"gpt-6-astra","claimed_model":"unified","tier":"fixed"}})), host).await.unwrap();
    peer.api("api/schedules", None, host).await.unwrap()["schedules"][0].clone()
}

fn batch_input() -> Value {
    json!({"name":"批量模型巡检","account_ids":["account-a","account-b"],"interval_minutes":60,"expected_model":"gpt-5.4","detection":{"method":"modeltrace","background":true,"account_id":"account-a","key_id":"key-b","request_model":"gpt-6-astra","claimed_model":"unified","tier":"fixed"}})
}

async fn batch_schedule(peer: &mut Peer, host: &mut Host) -> Value {
    peer.api("api/schedules/save", Some(batch_input()), host).await.unwrap();
    let item = peer.api("api/schedules", None, host).await.unwrap()["schedules"][0].clone();
    peer.api("api/schedules/control", Some(json!({"id":item["id"],"revision":1,"action":"enable"})), host).await.unwrap();
    item
}

#[tokio::test]
async fn manual_modeltrace_short_sample_still_requires_resume() {
    let mut peer = Peer::new().await;
    let mut host = Host {numeric_answer:true,numeric_counts:vec![51].into(),..Default::default()};
    let detection = batch_input()["detection"].clone();
    let run = peer.api("api/runs/start",Some(detection),&mut host).await.unwrap()["run"].clone();
    let mut bg = Background::new(peer,3_600_000).await;
    bg.until(&mut host, |h| saved(h,&run["id"])["status"] == "paused").await;
    assert_eq!(host.executions.len(),1);
    assert_eq!(saved(&host,&run["id"])["task_error"],"insufficient_numbers");
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn scheduled_short_samples_retry_within_budget_and_release_the_round() {
    for exhausted in [false, true] {
        let mut peer = Peer::new().await;
        let mut host = Host {
            numeric_answer: true,
            numeric_counts: if exhausted { vec![332,332,51,51,51] } else { vec![332,332,51] }.into(),
            ..Default::default()
        };
        batch_schedule(&mut peer, &mut host).await;
        let mut bg = Background::new(peer, 3_600_000).await;
        bg.until(&mut host, |h| {
            h.schedules["items"].0[0]["round"]["completed_at"].is_number()
                && h.state.values().all(|(r, _)| matches!(r["scheduled"]["alert"]["status"].as_str(), Some("sent" | "not_required")))
        }).await;
        let first = &host.state.values().find(|(r, _)| r["account_id"] == "account-a").unwrap().0;
        assert_eq!(first["status"], "completed");
        assert!(first["queued_at"].is_null() && first["inflight"].is_null());
        let attempts = first["attempts"].as_array().unwrap();
        assert_eq!(attempts.len(), if exhausted {5} else {4});
        assert_eq!(attempts[2]["parsed_numbers"], 51);
        assert_eq!(attempts[2]["error"], "insufficient_numbers");
        for good in &attempts[..2] {
            assert_eq!(attempts.iter().filter(|a| a["job_id"] == good["job_id"]).count(), 1);
        }
        assert_eq!(first["report"]["valid_samples"], if exhausted {2} else {3});
        assert_eq!(first["report"]["verdict"], if exhausted {"insufficient"} else {"attributed"});
        assert_eq!(host.notifications.len(), if exhausted {1} else {2});
        assert_eq!(host.executions.len(), if exhausted {8} else {7});
        let item = &host.schedules["items"].0[0];
        assert_eq!(item["next_run_at"].as_u64().unwrap(), item["round"]["completed_at"].as_u64().unwrap() + 3_600_000);
        bg.cancel(&mut host).await;
    }
}

#[tokio::test]
async fn scheduled_pause_and_stop_finish_current_sample_then_continue_other_accounts() {
    for action in ["pause", "stop"] {
        let mut peer = Peer::configured(json!({})).await;
        let mut host = Host { numeric_answer:true, numeric_counts:vec![51].into(), ..Default::default() };
        batch_schedule(&mut peer, &mut host).await;
        let mut bg = Background::new(peer, 3_600_000).await;
        bg.until(&mut host, |h| h.executions.len() == 1).await;
        let id = host.state.values().find(|(r,_)|r["account_id"]=="account-a").unwrap().0["id"].clone();
        bg.api("api/runs/control",Some(json!({"id":id,"action":action})),&mut host).await.unwrap();
        bg.until(&mut host, |h| h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
        let first = saved(&host,&id);
        assert_eq!(first["status"], "stopped");
        assert!(first["inflight"].is_null());
        assert_eq!(first["attempts"].as_array().unwrap().len(),1);
        assert_eq!(first["attempts"][0]["parsed_numbers"],51);
        assert_eq!(host.executions.len(),4);
        assert_eq!(host.executions.iter().filter(|e|e["account_id"]=="account-b").count(),3);
        assert!(host.schedules["items"].0[0]["next_run_at"].is_number());
        assert!(host.notifications.is_empty());
        bg.cancel(&mut host).await;
    }
}

#[tokio::test]
async fn batch_checks_every_account_immediately_and_times_from_last_completion() {
    let mut peer = Peer::new().await;
    let mut host = Host {numeric_answer:true, ..Default::default()};
    let item = batch_schedule(&mut peer, &mut host).await;
    let due = host.schedules["items"].0[0]["next_run_at"].clone();
    // 重复启用不推迟首轮，也不额外创建一轮
    peer.api("api/schedules/control", Some(json!({"id":item["id"],"revision":2,"action":"enable"})), &mut host).await.unwrap();
    assert_eq!(host.schedules["items"].0[0]["next_run_at"], due);
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| h.executions.len()==1).await;
    assert!(host.schedules["items"].0[0]["next_run_at"].is_null());
    assert_eq!(host.state.len(),2);
    bg.until(&mut host, |h| h.notifications.len()==2 && h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    assert_eq!(host.executions.len(),6);
    for account in ["account-a","account-b"] {
        assert_eq!(host.executions.iter().filter(|e|e["account_id"]==account).count(),3);
    }
    let completed = host.state.values().map(|(r,_)|r["updated_at"].as_u64().unwrap()).max().unwrap();
    let round = &host.schedules["items"].0[0];
    assert_eq!(round["round"]["completed_at"], completed);
    assert_eq!(round["next_run_at"], completed + 3_600_000);
    let list = bg.api("api/schedules",None,&mut host).await.unwrap();
    assert_eq!(list["schedules"][0]["round"]["members"].as_array().unwrap().len(),2);
    for member in list["schedules"][0]["round"]["members"].as_array().unwrap() {
        assert_eq!(member["run"]["status"],"completed");
        assert!(member["finished_at"].is_number());
    }
    assert_eq!(host.state.len(),2);
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn failed_batch_member_releases_round_and_allows_edit_and_delete_after_disable() {
    let mut peer = Peer::configured(json!({})).await;
    let mut host = Host {numeric_answer:true,execute_error:Some(PluginFault::new(ErrorCode::Upstream,"fixture rejection")),..Default::default()};
    let item = batch_schedule(&mut peer, &mut host).await;
    let mut bg = Background::new(peer,3_600_000).await;
    bg.until(&mut host, |h|h.executions.len()==1).await;
    bg.api("api/schedules/control",Some(json!({"id":item["id"],"revision":2,"action":"disable"})),&mut host).await.unwrap();
    bg.until(&mut host, |h|h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    assert_eq!(host.executions.len(),4);
    let first = &host.state.values().find(|(r,_)|r["account_id"]=="account-a").unwrap().0;
    assert_eq!(first["status"],"stopped");
    assert_eq!(first["task_error"],"fixture rejection");
    assert_eq!(first["scheduled"]["alert"]["status"],"not_required");
    let round = &host.schedules["items"].0[0];
    assert_eq!(round["round"]["members"][0]["error"],"fixture rejection");
    assert!(round["next_run_at"].is_null());
    let mut edit = batch_input(); edit["id"]=item["id"].clone(); edit["revision"]=json!(3);
    bg.api("api/schedules/save",Some(edit),&mut host).await.unwrap();
    bg.api("api/schedules/control",Some(json!({"id":item["id"],"revision":4,"action":"delete"})),&mut host).await.unwrap();
    assert_eq!(host.schedules["items"].0,json!([]));
    assert_eq!(host.state.len(),2);
    assert!(host.notifications.is_empty());
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn schedule_rejects_empty_duplicate_and_unavailable_account_selection() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    for accounts in [json!([]),json!(["account-a","account-a"]),json!(["account-a","missing"])] {
        let mut input=batch_input(); input["account_ids"]=accounts;
        assert!(peer.api("api/schedules/save",Some(input),&mut host).await.is_err());
    }
    assert!(host.schedules.is_empty() && host.state.is_empty() && host.executions.is_empty());
}

#[tokio::test]
async fn schedule_saves_all_47_accounts_and_completes_beyond_the_history_window() {
    let mut peer = Peer::configured(json!({})).await;
    let account_ids: Vec<_> = (0..47).map(|i| format!("account-{i}")).collect();
    let mut host = Host { numeric_answer: true, account_ids: account_ids.clone(), ..Default::default() };
    let mut input = batch_input();
    input["account_ids"] = json!(account_ids);
    peer.api("api/schedules/save", Some(input), &mut host).await.unwrap();
    let item = peer.api("api/schedules", None, &mut host).await.unwrap()["schedules"][0].clone();
    assert_eq!(item["account_ids"], json!(account_ids));
    assert_eq!(item["enabled"], false);
    assert!(host.state.is_empty() && host.executions.is_empty());
    peer.api("api/schedules/control", Some(json!({"id":item["id"],"revision":1,"action":"enable"})), &mut host).await.unwrap();

    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| h.executions.len() == 1).await;
    assert_eq!(host.state.len(), 16);
    assert!(host.schedules["items"].0[0]["next_run_at"].is_null());
    bg.until(&mut host, |h| h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    assert_eq!(host.executions.len(), 47 * 3);
    for id in &account_ids {
        assert_eq!(host.executions.iter().filter(|e| e["account_id"] == *id).count(), 3, "{id}");
    }
    assert_eq!(host.state.len(), 16);
    assert!(host.notifications.is_empty());
    let list = bg.api("api/schedules", None, &mut host).await.unwrap();
    let item = &list["schedules"][0];
    let members = item["round"]["members"].as_array().unwrap();
    assert_eq!(members.len(), 47);
    assert!(members.iter().all(|m| m["finished_at"].is_number() && m["error"].is_null()));
    assert_eq!(members.iter().filter(|m| m["run"].is_object()).count(), 47);
    let completed = members.iter().map(|m| m["finished_at"].as_u64().unwrap()).max().unwrap();
    assert_eq!(item["next_run_at"], completed + 3_600_000);
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn more_than_16_account_failures_do_not_block_a_47_account_round() {
    let mut peer = Peer::configured(json!({})).await;
    let account_ids: Vec<_> = (0..47).map(|i| format!("account-{i}")).collect();
    let mut host = Host { numeric_answer: true, account_ids: account_ids.clone(), ..Default::default() };
    let codes = [ErrorCode::PermissionDenied, ErrorCode::Timeout, ErrorCode::Upstream, ErrorCode::Rejected, ErrorCode::Fault];
    for (index, id) in account_ids[..20].iter().enumerate() {
        host.execute_errors.insert(id.clone(), PluginFault::new(codes[index % codes.len()], format!("fixture failure for {id}")));
    }
    let mut input = batch_input(); input["account_ids"] = json!(account_ids); input["enable"] = json!(true);
    peer.api("api/schedules/save", Some(input), &mut host).await.unwrap();
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    assert_eq!(host.executions.len(), 20 + 27 * 3);
    for (index, id) in account_ids.iter().enumerate() {
        assert_eq!(host.executions.iter().filter(|e| e["account_id"] == *id).count(), if index < 20 { 1 } else { 3 }, "{id}");
    }
    let list = bg.api("api/schedules", None, &mut host).await.unwrap();
    let item = &list["schedules"][0];
    let members = item["round"]["members"].as_array().unwrap();
    assert_eq!(members.len(), 47);
    assert!(members.iter().all(|m| m["finished_at"].is_number()));
    for (index, member) in members.iter().enumerate() {
        if index < 20 {
            assert_eq!(member["error"], format!("fixture failure for account-{index}"));
            assert_eq!(member["run"]["status"], "stopped", "archived failure report remains readable");
        } else {
            assert!(member["error"].is_null());
        }
    }
    assert_eq!(host.state.len(), 16);
    assert!(host.state.values().all(|(r,_)|r["status"]=="completed"));
    assert_eq!(item["next_run_at"].as_u64().unwrap(), item["round"]["completed_at"].as_u64().unwrap() + 3_600_000);
    assert!(host.notifications.is_empty());
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn schedule_enable_disable_edit_and_delete_preserve_explicit_control() {
    let mut peer = Peer::configured(json!({})).await;
    let mut host = Host::default();
    let item = scheduled(&mut peer, &mut host).await;
    assert_eq!(item["enabled"], false);
    assert!(item["next_run_at"].is_null() && host.executions.is_empty());
    let control = |action: &str, revision: u64| {
        Some(json!({"id":item["id"],"revision":revision,"action":action}))
    };
    peer.api("api/schedules/control", control("enable", 1), &mut host)
        .await
        .unwrap();
    let active = peer.api("api/schedules", None, &mut host).await.unwrap()["schedules"][0].clone();
    assert_eq!(active["enabled"], true);
    assert!(
        active["next_run_at"].as_u64().unwrap()
            <= std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
    );
    assert!(
        peer.api("api/schedules/control", control("disable", 1), &mut host)
            .await
            .is_err()
    );
    peer.api("api/schedules/control", control("disable", 2), &mut host)
        .await
        .unwrap();
    let mut bg = Background::new(peer, 3_600_000).await;
    let list = bg.api("api/schedules", None, &mut host).await.unwrap();
    assert_eq!(list["schedules"][0]["enabled"], false);
    assert!(list["schedules"][0]["next_run_at"].is_null() && host.executions.is_empty());
    let mut edit = item.clone();
    edit.as_object_mut().unwrap().retain(|k, _| {
        [
            "id",
            "revision",
            "name",
            "interval_minutes",
            "expected_model",
            "detection",
        ]
        .contains(&k.as_str())
    });
    edit["revision"] = json!(3);
    edit["interval_minutes"] = json!(5);
    bg.api("api/schedules/save", Some(edit), &mut host)
        .await
        .unwrap();
    bg.api("api/schedules/control", control("delete", 4), &mut host)
        .await
        .unwrap();
    assert!(
        bg.api("api/schedules", None, &mut host).await.unwrap()["schedules"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn due_schedule_runs_once_and_notifies_only_after_complete_report() {
    let mut peer = Peer::new().await;
    let mut host = Host {
        numeric_answer: true,
        ..Default::default()
    };
    let item = scheduled(&mut peer, &mut host).await;
    peer.api(
        "api/schedules/control",
        Some(json!({"id":item["id"],"revision":1,"action":"enable"})),
        &mut host,
    )
    .await
    .unwrap();
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| h.executions.len() == 1).await;
    assert!(host.notifications.is_empty());
    // 停用只影响后续调度，本轮继续完成并告警
    bg.api(
        "api/schedules/control",
        Some(json!({"id":item["id"],"revision":2,"action":"disable"})),
        &mut host,
    )
    .await
    .unwrap();
    bg.until(&mut host, |h| {
        h.state
            .values()
            .any(|(r, _)| r["scheduled"]["alert"]["status"] == "sent")
    })
    .await;
    assert_eq!(host.executions.len(), 3);
    assert_eq!(host.notifications.len(), 1);
    let run = &host.state.values().next().unwrap().0;
    assert_eq!(run["status"], "completed");
    assert_eq!(run["report"]["valid_samples"], 3);
    let text = host.notifications[0]["content"]["text"].as_str().unwrap();
    assert!(
        text.contains("模型检测告警") && text.contains("gpt-6-astra") && text.contains("gpt-5.4")
    );
    assert!(!text.contains("test-id") && !text.contains("key-b"));
    let peer = bg.cancel(&mut host).await;
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.api("api/schedules", None, &mut host).await.unwrap();
    let peer = bg.cancel(&mut host).await;
    drop(peer);
    assert_eq!(host.notifications.len(), 1);
    assert_eq!(host.state.len(), 1);
}

#[tokio::test]
async fn paused_schedule_slots_after_restart_do_not_block_the_remaining_31_accounts() {
    let mut peer = Peer::configured(json!({})).await;
    let account_ids: Vec<_> = (0..47).map(|i|format!("account-{i}")).collect();
    let mut host = Host {numeric_answer:true,account_ids:account_ids.clone(),..Default::default()};
    let mut input = batch_input(); input["account_ids"]=json!(account_ids); input["enable"]=json!(true);
    peer.api("api/schedules/save",Some(input),&mut host).await.unwrap();
    let mut bg = Background::new(peer,3_600_000).await;
    bg.until(&mut host, |h|h.state.len()==16).await;
    drop(bg.cancel(&mut host).await);
    assert!(host.executions.is_empty());
    for (run,_) in host.state.values_mut() {
        run["status"]=json!("paused"); run["queued_at"]=Value::Null;
        run["task_error"]=json!("fixture previously paused account");
    }
    host.schedules.get_mut("items").unwrap().0[0]["enabled"]=json!(false);
    host.schedules.get_mut("items").unwrap().0[0]["next_run_at"]=Value::Null;
    let mut bg = Background::new(Peer::configured(json!({})).await,3_600_000).await;
    bg.until(&mut host, |h|h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    let members=host.schedules["items"].0[0]["round"]["members"].as_array().unwrap();
    assert_eq!(members.len(),47);
    assert_eq!(members.iter().filter(|m|m["error"]=="fixture previously paused account").count(),16);
    assert!(members.iter().all(|m|m["finished_at"].is_number()));
    assert_eq!(host.executions.len(),31*3);
    for id in &account_ids[..16] {assert!(!host.executions.iter().any(|e|e["account_id"]==*id));}
    assert_eq!(host.state.len(),16);
    assert!(host.schedules["items"].0[0]["next_run_at"].is_null());
    assert!(host.notifications.is_empty());
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn interrupted_scheduled_account_does_not_discard_other_queued_accounts() {
    let mut peer = Peer::configured(json!({})).await;
    let account_ids: Vec<_> = (0..47).map(|i| format!("account-{i}")).collect();
    let mut host = Host { numeric_answer: true, account_ids: account_ids.clone(), ..Default::default() };
    let mut input = batch_input(); input["account_ids"]=json!(account_ids); input["enable"]=json!(true);
    peer.api("api/schedules/save",Some(input),&mut host).await.unwrap();
    let mut bg = Background::new(peer,3_600_000).await;
    bg.until(&mut host, |h|h.executions.len()==1).await;
    assert_eq!(host.state.len(),16);
    drop(bg.cancel(&mut host).await);
    let mut bg = Background::new(Peer::configured(json!({})).await,3_600_000).await;
    bg.until(&mut host, |h|h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    assert_eq!(host.executions.len(),1+46*3);
    for (index,id) in account_ids.iter().enumerate() {
        assert_eq!(host.executions.iter().filter(|e|e["account_id"]==*id).count(),if index==0 {1} else {3});
    }
    let members=host.schedules["items"].0[0]["round"]["members"].as_array().unwrap();
    assert!(members[0]["error"].is_string());
    assert!(members[1..].iter().all(|m|m["error"].is_null() && m["finished_at"].is_number()));
    assert!(host.notifications.is_empty());
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn full_history_waits_for_capacity_and_finishes_each_batch_account_once() {
    let mut peer = Peer::configured(json!({})).await;
    let mut host = Host {numeric_answer:true,..Default::default()};
    let template=queued(&mut peer,&mut host).await;
    host.state.clear();
    for slot in 0..16 {
        let mut run=template.clone();
        run["id"]=json!(format!("{slot}-1")); run["slot"]=json!(slot);
        run["status"]=json!("paused"); run["queued_at"]=Value::Null;
        host.state.insert(format!("run-{slot}"),(run,1));
    }
    batch_schedule(&mut peer,&mut host).await;
    let mut bg=Background::new(peer,3_600_000).await;
    bg.until(&mut host, |h|h.schedules["items"].0[0]["error"].is_string()).await;
    assert!(host.executions.is_empty());
    assert!(host.schedules["items"].0[0]["next_run_at"].is_null());
    bg.api("api/runs/control",Some(json!({"id":"0-1","action":"stop"})),&mut host).await.unwrap();
    bg.until(&mut host, |h|h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    assert_eq!(host.executions.len(),6);
    assert_eq!(host.state.len(),16);
    for account in ["account-a","account-b"] {
        assert_eq!(host.executions.iter().filter(|e|e["account_id"]==account).count(),3);
    }
    // 单个空位轮流使用，首个报告可回收，已完成进度仍然保留
    assert!(host.schedules["items"].0[0]["round"]["members"].as_array().unwrap().iter().all(|m|m["finished_at"].is_number()));
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn optional_alerts_preserve_reports_history_and_atomic_save_enable_across_rounds() {
    let mut peer = Peer::configured(json!({})).await;
    let mut host = Host { numeric_answer: true, ..Default::default() };
    let mut input = batch_input();
    input["enable"] = json!(true);
    let mut invalid = input.clone();
    invalid["account_ids"] = json!(["missing"]);
    assert!(peer.api("api/schedules/save", Some(invalid), &mut host).await.is_err());
    assert!(host.schedules.is_empty() && host.state.is_empty());
    peer.api("api/schedules/save", Some(input.clone()), &mut host).await.unwrap();
    let id = host.schedules["items"].0[0]["id"].clone();
    assert_eq!(host.schedules["items"].0[0]["enabled"], true);
    assert!(host.schedules["items"].0[0]["next_run_at"].is_number());
    let mut bg = Background::new(peer, 3_600_000).await;
    for count in [2, 4] {
        bg.until(&mut host, |h| {
            h.state.len() == count
                && h.state.values().all(|(r, _)| r["scheduled"]["alert"]["status"] == "skipped")
                && h.schedules["items"].0[0]["round"]["completed_at"].is_number()
        }).await;
        assert!(host.notifications.is_empty());
        assert_eq!(host.executions.len(), count * 3);
        let schedule = &host.schedules["items"].0[0];
        assert_eq!(schedule["next_run_at"].as_u64().unwrap(), schedule["round"]["completed_at"].as_u64().unwrap() + 3_600_000);
        let history = bg.api("api/runs", None, &mut host).await.unwrap();
        let runs = history["runs"].as_array().unwrap();
        assert_eq!(runs.len(), count);
        for run in runs {
            assert_eq!(run["status"], "completed");
            assert_eq!(run["report"]["valid_samples"], 3);
            assert_eq!(run["scheduled"]["schedule_id"], id);
            assert_eq!(run["scheduled"]["alert"]["status"], "skipped");
            assert_eq!(run["scheduled"]["alert"]["attempts"], 0);
            assert!(run["scheduled"]["due_at"].is_number());
            let report = bg.api("api/runs/get", Some(json!({"id":run["id"]})), &mut host).await.unwrap();
            assert_eq!(report["run"]["report"], run["report"]);
        }
        if count == 2 {
            bg.api("api/schedules/control", Some(json!({"id":id,"revision":1,"action":"disable"})), &mut host).await.unwrap();
            input["id"] = id.clone(); input["revision"] = json!(2);
            bg.api("api/schedules/save", Some(input.clone()), &mut host).await.unwrap();
        }
    }
    drop(bg.cancel(&mut host).await);
    // 后来配置飞书只告警新报告，不补发已明确跳过的历史结果
    let mut bg = Background::new(Peer::new().await, 3_600_000).await;
    bg.api("api/schedules/control", Some(json!({"id":id,"revision":3,"action":"run_now"})), &mut host).await.unwrap();
    bg.until(&mut host, |h| h.notifications.len() == 2 && h.schedules["items"].0[0]["round"]["completed_at"].is_number()).await;
    assert_eq!(host.executions.len(), 18);
    assert_eq!(host.state.values().filter(|(r, _)| r["scheduled"]["alert"]["status"] == "skipped").count(), 4);
    bg.cancel(&mut host).await;
}

#[tokio::test]
async fn failed_feishu_response_is_not_success_and_retry_budget_survives_restart() {
    let mut peer = Peer::new().await;
    let mut host = Host {
        numeric_answer: true,
        ..Default::default()
    };
    let item = scheduled(&mut peer, &mut host).await;
    peer.api(
        "api/schedules/control",
        Some(json!({"id":item["id"],"revision":1,"action":"enable"})),
        &mut host,
    )
    .await
    .unwrap();
    host.schedules.get_mut("items").unwrap().0[0]["next_run_at"] = json!(0);
    host.http
        .push_back((200, br#"{"code":19024,"msg":"fixture-secret"}"#.to_vec()));
    let mut bg = Background::new(peer, 3_600_000).await;
    bg.until(&mut host, |h| {
        h.state
            .values()
            .any(|(r, _)| r["scheduled"]["alert"]["error"].is_string())
    })
    .await;
    let delivery = &host.state.values().next().unwrap().0["scheduled"]["alert"];
    assert_eq!(delivery["status"], "pending");
    assert!(!delivery.to_string().contains("fixture-secret"));
    bg.cancel(&mut host).await;
    for attempt in 2..=3 {
        host.state.values_mut().next().unwrap().0["scheduled"]["alert"]["next_attempt_at"] =
            json!(0);
        host.http.push_back((503, vec![]));
        let mut bg = Background::new(Peer::new().await, 3_600_000).await;
        bg.until(&mut host, |h| {
            h.state.values().any(|(r, _)| {
                r["scheduled"]["alert"]["attempts"] == attempt
                    && r["scheduled"]["alert"]["status"] != "sending"
            })
        })
        .await;
        bg.cancel(&mut host).await;
    }
    assert_eq!(host.notifications.len(), 3);
    assert_eq!(
        host.state.values().next().unwrap().0["scheduled"]["alert"]["status"],
        "failed"
    );
    assert_eq!(host.executions.len(), 3);
}

#[tokio::test]
async fn schedule_commit_failure_recovers_existing_round_without_duplicate_creation() {
    let mut peer = Peer::new().await;
    let mut host = Host::default();
    batch_schedule(&mut peer, &mut host).await;
    host.schedules.get_mut("items").unwrap().0[0]["next_run_at"] = json!(0);
    host.reject_schedule_commit = true;
    let mut bg = Background::new(peer, 3_600_000).await;
    assert!(bg.finish(&mut host).await.is_err());
    assert_eq!(host.state.len(), 2);
    assert!(host.executions.is_empty());
    let created: Vec<_> = host.state.values().map(|(r,_)|r["id"].clone()).collect();
    assert!(host.state.values().all(|(r,_)|r["scheduled"]["awaiting_checkpoint"]==true));
    let mut bg = Background::new(bg.peer, 3_600_000).await;
    bg.until(&mut host, |h| {
        h.schedules["items"].0[0]["round"]["members"].as_array().unwrap().iter().all(|m|created.contains(&m["run_id"]))
    })
    .await;
    bg.cancel(&mut host).await;
    assert_eq!(host.state.len(), 2);
    assert!(host.executions.is_empty());
    assert_eq!(host.state.values().next().unwrap().0["status"], "ready");
}
