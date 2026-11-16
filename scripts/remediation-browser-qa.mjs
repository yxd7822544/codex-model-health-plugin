import {createRequire} from 'node:module'
import assert from 'node:assert/strict'
import {mkdirSync} from 'node:fs'
import {fileURLToPath} from 'node:url'
const require=createRequire(import.meta.url)
const {chromium}=require(process.env.PLAYWRIGHT_PATH || 'playwright')
const base=process.env.PREVIEW_URL || 'http://127.0.0.1:4175/'
const root=fileURLToPath(new URL('../',import.meta.url))
mkdirSync(`${root}/review/screenshots`,{recursive:true})
const browser=await chromium.launch({headless:true,executablePath:process.env.CHROME_BINARY})
try {
  const seedPage=await browser.newPage();await seedPage.goto(`${base}?preview=1`)
  await seedPage.getByRole('heading',{name:'任务列表',exact:true}).waitFor()
  const {boot,seed}=await seedPage.evaluate(async()=>{
    const read=async(path,body)=>JSON.parse(new TextDecoder().decode((await window.codexProxyPlugin.request({path:`api/${path}`,method:body?'POST':'GET',...(body?{body:JSON.stringify(body),contentType:'application/json'}:{})})).body))
    return {boot:await read('bootstrap'),seed:(await read('runs/get',{id:'0-1'})).run}
  })
  const theme=await seedPage.locator('html').evaluate(e=>{const s=getComputedStyle(e);return Array.from(s).filter(n=>n.startsWith('--')).map(n=>`${n}:${s.getPropertyValue(n)}`).join(';')});await seedPage.close()
  let phase='pending'
  const retries=[]
  const action=status=>({status,attempts:status==='pending'?0:1,error:status==='failed'?'宿主改组接口失败（HTTP 503）':null,group_id:status==='succeeded'?'group-downgraded':null,completed_at:status==='succeeded'?Date.now():null})
  const records=()=>[0,1].map(index=>{
    const finished=index===0 || phase==='empty'
    return {...seed,id:`${index}-1`,account_id:index?'demo-b':'demo-a',account_name:index?'备用账号':'主账号',status:finished?'completed':'ready',background:true,queued_at:finished?null:1,inflight:null,scheduled:{schedule_id:'plan',name:'自动移组巡检',due_at:1,expected_model:'gpt-5.6-sol',alert:{status:'not_required',attempts:0,error:null},remediation:action(phase==='empty' || (index===0 && phase==='success')?'succeeded':index===0 && phase==='failed'?'failed':'pending')}}
  })
  const schedule=()=>({id:'plan',name:'自动移组巡检',revision:1,enabled:phase!=='empty',account_ids:phase==='empty'?[]:phase==='success'?['demo-b']:['demo-a','demo-b'],interval_minutes:180,expected_model:'gpt-5.6-sol',detection:{method:'meow',request_model:'gpt-6-astra'},next_run_at:null,last_run:null,error:phase==='empty'?'检测账号已全部移至 GPT降智分组，计划已停用':null,round:{due_at:1,completed_at:phase==='empty'?Date.now():null,members:records().map(run=>({account_id:run.account_id,run_id:run.id,finished_at:run.status==='completed'?Date.now():null,error:null,run:{...run,alert:run.scheduled.alert}}))}})
  const context=await browser.newContext({viewport:{width:1500,height:1200}})
  await context.exposeBinding('__remediationHost',(_,input)=>{
    if(input.path==='api/bootstrap')return boot
    if(input.path==='api/schedules')return {schedules:[schedule()],alerts_configured:false,remediation_configured:true}
    if(input.path==='api/runs')return {runs:records().map(run=>({...run,method:'meow',tier:'low',request_model:'gpt-6-astra',attempts:run.attempts.length})),total:2,page:1,page_size:JSON.parse(input.body).page_size}
    if(input.path==='api/runs/get')return {run:records().find(run=>run.id===JSON.parse(input.body).id)}
    if(input.path==='api/runs/remediation/retry') {
      assert.equal(input.method,'POST')
      assert.deepEqual(JSON.parse(input.body),{id:'0-1'})
      retries.push(JSON.parse(input.body).id);phase='success'
      return {run:records()[0]}
    }
    throw new Error(`unexpected mutation ${input.path}`)
  })
  await context.addInitScript(()=>{window.codexProxyPlugin={version:2,request:async input=>({status:200,contentType:'application/json',body:new TextEncoder().encode(JSON.stringify(await window.__remediationHost(input))).buffer})}})
  const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));await page.goto(base);await page.locator('html').evaluate((e,s)=>e.setAttribute('style',s),theme)
  const row=page.getByRole('row').filter({has:page.getByText('自动移组巡检',{exact:true})})
  await row.getByText('后续检测 2 个账号',{exact:true}).waitFor()
  await row.getByRole('button',{name:'查看本轮',exact:true}).click()
  const dialog=page.getByRole('dialog');await dialog.getByText('等待自动改组',{exact:true}).waitFor()
  phase='success'
  await dialog.getByText('已移入 GPT降智分组，并移出本计划',{exact:true}).waitFor({timeout:10000})
  assert.equal(await dialog.getByRole('row').count(),3,'frozen round still contains both accounts')
  await dialog.press('Escape');await dialog.waitFor({state:'hidden'})
  await row.getByText('后续检测 1 个账号',{exact:true}).waitFor()
  phase='failed';await row.getByText('后续检测 2 个账号',{exact:true}).waitFor({timeout:10000})
  await row.getByRole('button',{name:'查看本轮',exact:true}).click()
  await dialog.getByText('自动改组失败',{exact:true}).waitFor();await dialog.getByText('宿主改组接口失败（HTTP 503）',{exact:true}).waitFor()
  assert.equal(await dialog.getByRole('button',{name:'重试改组',exact:true}).count(),1,'unfinished accounts cannot retry grouping')
  await dialog.getByRole('button',{name:'重试改组',exact:true}).click()
  await dialog.getByText('已移入 GPT降智分组，并移出本计划',{exact:true}).waitFor()
  assert.equal(await dialog.getByRole('row').count(),3,'retry preserves the full round')
  await dialog.press('Escape');await dialog.waitFor({state:'hidden'})
  await row.getByText('后续检测 1 个账号',{exact:true}).waitFor()
  phase='failed'
  await page.getByRole('button',{name:'查看 主账号 的检测报告',exact:true}).click()
  const report=page.getByRole('region',{name:'检测报告'})
  await report.getByText(/自动改组失败/).waitFor()
  await report.getByRole('button',{name:'重试改组',exact:true}).click()
  await report.getByText('已移入 GPT降智分组，并移出本计划',{exact:true}).waitFor()
  assert.deepEqual(retries,['0-1','0-1'],'both entry points only call remediation retry')
  phase='empty';await row.getByText('后续检测 0 个账号',{exact:true}).waitFor({timeout:10000})
  assert.equal(await row.getByRole('button',{name:'启用',exact:true}).isDisabled(),true)
  await row.getByText('已结束 2 / 2 个账号',{exact:true}).waitFor()
  await page.getByRole('button',{name:'查看 主账号 的检测报告',exact:true}).click()
  await page.getByRole('region',{name:'检测报告'}).getByText('已移入 GPT降智分组，并移出本计划',{exact:true}).waitFor()
  await row.screenshot({path:`${root}/review/screenshots/remediation-empty-plan.png`})
  assert.deepEqual(errors,[])
  console.log('Remediation UI passed: both retry buttons, no model rerun, live account count, unchanged round members, failure reasons, report status, empty-plan disabled state.')
} finally {await browser.close()}
