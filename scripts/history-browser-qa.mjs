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
  const seedPage=await browser.newPage()
  await seedPage.goto(`${base}?preview=1`)
  await seedPage.getByRole('heading',{name:'任务列表',exact:true}).waitFor()
  const {boot,seed}=await seedPage.evaluate(async()=>{
    const read=async(path,body)=>JSON.parse(new TextDecoder().decode((await window.codexProxyPlugin.request({path:`api/${path}`,method:body?'POST':'GET',...(body?{body:JSON.stringify(body),contentType:'application/json'}:{})})).body))
    return {boot:await read('bootstrap'),seed:(await read('runs/get',{id:'0-1'})).run}
  })
  const theme=await seedPage.locator('html').evaluate(e=>{const style=getComputedStyle(e);return Array.from(style).filter(name=>name.startsWith('--')).map(name=>`${name}:${style.getPropertyValue(name)}`).join(';')})
  await seedPage.close()
  const rows=Array.from({length:1024},(_,i)=>({...seed,id:`0-${1024-i}`,account_name:`历史账号 ${1024-i}`,created_at:1024-i,method:'meow',tier:'low',request_model:'gpt-6-astra',claimed_model:'gpt-6-astra',attempts:36,planned:36,scheduled:i%2===0?{schedule_id:'plan-a',name:'计划 A',due_at:0,expected_model:'gpt-6-astra',alert:{status:'not_required',attempts:0,error:null}}:null}))
  let holdPage,release,held=false
  const calls=[],errors=[]
  const context=await browser.newContext({viewport:{width:1440,height:1100}})
  await context.exposeBinding('__historyHost',async(_,input)=>{
    const data=input.body?JSON.parse(input.body):{}
    if(input.path==='api/bootstrap')return boot
    if(input.path==='api/schedules')return {alerts_configured:false,schedules:[{id:'plan-a',revision:1,name:'计划 A',account_ids:['demo-a'],enabled:false,interval_minutes:60,expected_model:'gpt-6-astra',detection:{method:'meow',request_model:'gpt-6-astra'},round:null,last_run:null,next_run_at:null,error:null}]}
    if(input.path==='api/runs'){
      assert.equal(input.method,'POST');calls.push(data)
      const filtered=rows.filter(r=>!data.schedule_id || r.scheduled?.schedule_id===data.schedule_id)
      const page=Math.min(data.page,Math.max(1,Math.ceil(filtered.length/data.page_size)))
      const value={runs:filtered.slice((page-1)*data.page_size,page*data.page_size),total:filtered.length,page,page_size:data.page_size}
      if(holdPage===page){held=true;holdPage=undefined;await new Promise(resolve=>{release=resolve})}
      return value
    }
    if(input.path==='api/runs/get'){const row=rows.find(r=>r.id===data.id);return {run:{...seed,id:row.id,account_name:row.account_name}}}
    throw new Error(`unexpected request ${input.path}`)
  })
  await context.addInitScript(()=>{window.codexProxyPlugin={version:2,request:async input=>({status:200,contentType:'application/json',body:new TextEncoder().encode(JSON.stringify(await window.__historyHost(input))).buffer})}})
  const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message))
  await page.goto(base);await page.locator('html').evaluate((e,style)=>e.setAttribute('style',style),theme)
  const history=page.getByRole('region',{name:'检测执行记录'})
  const names=()=>history.getByRole('row').filter({has:page.getByRole('button',{name:/查看 历史账号/})})
  await history.getByRole('button',{name:'查看 历史账号 1024 的检测报告',exact:true}).waitFor()
  assert.equal(await names().count(),20)
  await history.getByRole('button',{name:'下一页',exact:true}).click()
  await history.getByRole('button',{name:'查看 历史账号 1004 的检测报告',exact:true}).waitFor()
  await history.getByRole('button',{name:'查看 历史账号 1004 的检测报告',exact:true}).click()
  await page.getByRole('heading',{name:'历史账号 1004',exact:true}).waitFor()
  await history.getByRole('button',{name:'上一页',exact:true}).click()
  await history.getByRole('button',{name:'查看 历史账号 1024 的检测报告',exact:true}).waitFor()
  await page.waitForTimeout(2200)
  assert.equal(await page.getByRole('heading',{name:'历史账号 1004',exact:true}).count(),1,'report remains open on a different page')
  const size=history.getByRole('combobox',{name:'每页条数',exact:true});await size.click()
  const list=page.locator(`[id="${await size.getAttribute('aria-controls')}"]`)
  await list.getByRole('option',{name:/100/}).click()
  await page.waitForFunction(()=>document.querySelectorAll('section[aria-label="检测执行记录"] tbody tr').length===100)
  await history.getByRole('button',{name:'11',exact:true}).click()
  await history.getByRole('button',{name:'查看 历史账号 24 的检测报告',exact:true}).waitFor()
  assert.equal(await names().count(),24)
  await page.getByRole('button',{name:'执行历史',exact:true}).click()
  await history.getByRole('button',{name:'查看 历史账号 1024 的检测报告',exact:true}).waitFor()
  assert(calls.at(-1).schedule_id==='plan-a' && calls.at(-1).page===1)
  await history.getByText('共 512 条',{exact:true}).first().waitFor()
  await history.getByRole('button',{name:'显示全部',exact:true}).click()
  await history.getByText('共 1024 条',{exact:true}).first().waitFor()
  holdPage=1
  while(!held)await new Promise(resolve=>setTimeout(resolve,50))
  await history.getByRole('button',{name:'下一页',exact:true}).click()
  await history.getByRole('button',{name:'查看 历史账号 924 的检测报告',exact:true}).waitFor()
  release();await page.waitForTimeout(200)
  assert.equal(await history.getByRole('button',{name:'查看 历史账号 1024 的检测报告',exact:true}).count(),0,'late old-page poll cannot replace current page')
  await history.screenshot({path:`${root}/review/screenshots/history-pagination.png`})
  assert.deepEqual(errors,[])
  console.log('History UI passed: 1024 records, page sizes, last page, plan filter, cross-page report, late polling response.')
} finally {await browser.close()}
