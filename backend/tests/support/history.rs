use super::*;
use base64::{Engine as _,engine::general_purpose::STANDARD};
use flate2::{Compression,write::GzEncoder};

pub(super) fn encoded(run:&Value)->Value {
    let mut writer=GzEncoder::new(Vec::new(),Compression::default());
    serde_json::to_writer(&mut writer,run).unwrap();
    json!({"data":STANDARD.encode(writer.finish().unwrap())})
}
fn record(template:&Value,slot:usize,number:usize)->Value {
    let mut run=template.clone();
    run["id"]=json!(format!("{slot}-{number}"));run["slot"]=json!(slot);
    run["created_at"]=json!(number);run["updated_at"]=json!(number+1);
    run["status"]=json!("stopped");run["queued_at"]=Value::Null;run["inflight"]=Value::Null;
    run["attempts"]=json!([{"job_id":"sample-1","status":"error","error":"fixture archived error","request_id":format!("request-{number}"),"send_state":"sent"}]);
    run["task_error"]=json!("fixture archived error");run["scheduled"]=Value::Null;
    run
}
async fn seed(peer:&mut Peer,host:&mut Host)->Value {
    let template=peer.start(host).await;host.state.clear();
    for number in 0..1008 {
        let mut run=record(&template,number%16,number);
        if number%3==0 {run["scheduled"]=json!({"schedule_id":"fixture-plan","name":"归档计划","due_at":0,"expected_model":"gpt-6-astra","alert":{"status":"not_required","attempts":0,"next_attempt_at":0,"error":null},"awaiting_checkpoint":false});}
        host.archive.insert(format!("record-{number}"),(encoded(&run),1));
    }
    for slot in 0..16 {host.state.insert(format!("run-{slot}"),(record(&template,slot,2000+slot),1));}
    template
}
async fn page(peer:&mut Peer,host:&mut Host,number:usize,size:usize,plan:Option<&str>)->Value {
    peer.api("api/runs",Some(json!({"page":number,"page_size":size,"schedule_id":plan})),host).await.unwrap()
}

#[tokio::test]
async fn over_1000_history_records_page_filter_and_reopen_without_full_rescans() {
    let mut peer=Peer::new().await;let mut host=Host::default();let template=seed(&mut peer,&mut host).await;
    let first=page(&mut peer,&mut host,1,20,None).await;
    assert_eq!(first["total"],1024);assert_eq!(first["runs"].as_array().unwrap().len(),20);
    let mut ids=BTreeSet::new();
    let mut times=vec![];
    for number in 1..=52 {
        let before=host.callbacks.len();let result=page(&mut peer,&mut host,number,20,None).await;
        assert_eq!(result["page"],number);
        assert!(host.callbacks.len()-before<=40,"page must only read active slots and requested archive records");
        for run in result["runs"].as_array().unwrap() {assert!(ids.insert(run["id"].as_str().unwrap().to_owned()));times.push(run["created_at"].as_u64().unwrap());}
    }
    assert_eq!(ids.len(),1024);assert!(times.windows(2).all(|pair|pair[0]>=pair[1]));
    let filtered=page(&mut peer,&mut host,2,50,Some("fixture-plan")).await;
    assert_eq!(filtered["total"],336);assert_eq!(filtered["runs"].as_array().unwrap().len(),50);
    assert!(filtered["runs"].as_array().unwrap().iter().all(|r|r["scheduled"]["schedule_id"]=="fixture-plan"));
    let detail=peer.api("api/runs/get",Some(json!({"id":"3-99"})),&mut host).await.unwrap()["run"].clone();
    let original=record(&template,3,99);
    for field in ["id","plan","attempts","report","task_error"] {assert_eq!(detail[field],original[field]);}
    assert!(peer.api("api/runs/control",Some(json!({"id":"3-99","action":"resume"})),&mut host).await.is_err());
    drop(peer);let mut peer=Peer::new().await;
    assert_eq!(page(&mut peer,&mut host,500,100,None).await["page"],11);
    assert_eq!(page(&mut peer,&mut host,1,20,Some("missing-plan")).await["total"],0);
    assert!(host.executions.is_empty());
}

#[tokio::test]
async fn archive_before_reuse_is_durable_deduplicated_and_evicts_only_oldest_history() {
    let mut peer=Peer::new().await;let mut host=Host::default();seed(&mut peer,&mut host).await;
    let original=host.state["run-0"].0.clone();
    let input=json!({"account_id":"account-b","key_id":"key-b","request_model":"gpt-6-astra","claimed_model":"gpt-6-astra","tier":"low"});
    host.reject_archive_write=true;
    assert!(peer.api("api/runs/start",Some(input.clone()),&mut host).await.is_err());
    assert_eq!(host.state["run-0"].0,original);
    host.reject_archive_write=false;host.reject_slot_replacement=true;
    assert!(peer.api("api/runs/start",Some(input.clone()),&mut host).await.is_err());
    assert_eq!(host.state["run-0"].0,original);assert_eq!(host.archive.len(),1008);
    assert_eq!(page(&mut peer,&mut host,1,20,None).await["total"],1023,"active/archive duplicate is counted once");
    drop(peer);let mut peer=Peer::new().await;host.reject_slot_replacement=false;
    let new=peer.api("api/runs/start",Some(input),&mut host).await.unwrap()["run"].clone();
    assert_eq!(new["slot"],0);assert_eq!(page(&mut peer,&mut host,1,20,None).await["total"],1024);
    assert_eq!(host.archive.len(),1008);
    assert!(peer.api("api/runs/get",Some(json!({"id":"0-0"})),&mut host).await.is_err());
    let old=peer.api("api/runs/get",Some(json!({"id":"0-2000"})),&mut host).await.unwrap();
    for field in ["plan","attempts","report","task_error"] {assert_eq!(old["run"][field],original[field]);}
    assert!(host.executions.is_empty());
}

#[tokio::test]
async fn history_rejects_invalid_page_parameters() {
    let mut peer=Peer::new().await;let mut host=Host::default();
    for input in [json!({"page":0,"page_size":20}),json!({"page":1,"page_size":1000}),json!({"page":1,"page_size":20,"schedule_id":""})] {
        assert!(peer.api("api/runs",Some(input),&mut host).await.is_err());
    }
    assert!(host.executions.is_empty() && host.state.is_empty() && host.archive.is_empty());
}

#[test]
fn history_manifest_has_room_for_at_least_1000_complete_archives() {
    let manifest:Value=serde_json::from_slice(include_bytes!("../../../plugin.json")).unwrap();
    let shards:Vec<_>=manifest["state"].as_array().unwrap().iter().filter(|s|s["namespace"].as_str().unwrap().starts_with("history-")).collect();
    assert_eq!(shards.iter().map(|s|s["maximumRecords"].as_u64().unwrap()).sum::<u64>(),1008);
    for shard in shards {assert!(shard["maximumBytes"].as_u64().unwrap()>=shard["maximumRecords"].as_u64().unwrap()*shard["maximumValueBytes"].as_u64().unwrap());}
}
