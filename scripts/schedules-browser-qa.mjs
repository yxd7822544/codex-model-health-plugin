// Exercise the actual schedule editor against a host with more accounts than history slots.
import { createRequire } from 'node:module'
import assert from 'node:assert/strict'
import { mkdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const { chromium } = require(process.env.PLAYWRIGHT_PATH || 'playwright')
const base = process.env.PREVIEW_URL || 'http://127.0.0.1:4175/'
const root = fileURLToPath(new URL('../', import.meta.url))
mkdirSync(`${root}/review/screenshots`, { recursive: true })
const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME_BINARY })
try {
  const seed = await browser.newPage()
  await seed.goto(`${base}?preview=1`)
  await seed.getByRole('heading', { name: '定时任务', exact: true }).waitFor()
  const boot = await seed.evaluate(async () => JSON.parse(new TextDecoder().decode((await window.codexProxyPlugin.request({ method: 'GET', path: 'api/bootstrap' })).body)))
  const theme = await seed.locator('html').evaluate(element => {
    const styles = getComputedStyle(element)
    return Array.from(styles).filter(name => name.startsWith('--')).map(name => `${name}:${styles.getPropertyValue(name)}`).join(';')
  })
  await seed.close()
  let accounts = Array.from({ length: 47 }, (_, i) => ({ id: `account-${i}`, name: 'Codex', email: `codex-${i}@example.com`, plan: 'pro' }))
  let schedules = []
  let failGroups = false
  const accountGroups = () => [
    {id:'group-a',name:'线上账号',enabled:true,account_ids:accounts.slice(0,20).map(a=>a.id)},
    {id:'group-b',name:'巡检账号',enabled:true,account_ids:accounts.slice(10,35).map(a=>a.id)},
    {id:'empty',name:'空分组',enabled:true,account_ids:[]},
    {id:'',name:'未分组',enabled:true,account_ids:accounts.slice(35).map(a=>a.id)},
  ]
  const saves = [], errors = []
  const context = await browser.newContext({ viewport: { width: 1440, height: 1050 } })
  await context.exposeBinding('__scheduleHost', (_, input) => {
    if (input.path === 'api/bootstrap') return { ...boot, accounts }
    if (input.path === 'api/account-groups') {
      if (failGroups) throw new Error('模拟分组读取失败')
      return {groups: accountGroups()}
    }
    if (input.path === 'api/models') return { models: ['gpt-6-astra'] }
    if (input.path === 'api/runs') return { runs: [], total:0, page:1, page_size:JSON.parse(input.body).page_size }
    if (input.path === 'api/schedules') return { schedules, alerts_configured: false }
    if (input.path === 'api/schedules/save') {
      const data = JSON.parse(input.body)
      saves.push(data)
      schedules = [{ ...data, id: 'schedule-47', revision: 1, enabled: false, next_run_at: null, last_run_id: null, last_run: null, round: null, error: null }]
      return { saved: true }
    }
    if (input.path === 'api/schedules/control') {
      const data = JSON.parse(input.body)
      assert.equal(data.action, 'delete')
      schedules = schedules.filter(item => item.id !== data.id)
      return { saved: true }
    }
    throw new Error(`unexpected page call: ${input.path}`)
  })
  await context.addInitScript(() => {
    window.codexProxyPlugin = { version: 2, request: async input => ({ status: 200, contentType: 'application/json', body: new TextEncoder().encode(JSON.stringify(await window.__scheduleHost(input))).buffer }) }
  })
  const page = await context.newPage()
  page.on('pageerror', error => errors.push(error.message))
  await page.goto(base)
  await page.locator('html').evaluate((element, style) => element.setAttribute('style', style || ''), theme)
  await page.getByRole('button', { name: '新建定时任务', exact: true }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByText('已选 0 / 47', { exact: true }).waitFor()
  await dialog.getByRole('checkbox', { name: '线上账号（20 个账号）', exact: true }).waitFor()
  assert.equal(await dialog.getByRole('checkbox', { name: '空分组（0 个账号）', exact: true }).isDisabled(), true)
  await dialog.getByText('线上账号（20 个账号）', { exact: true }).click()
  await dialog.getByText('已选 20 / 47', { exact: true }).waitFor()
  await dialog.getByText('巡检账号（25 个账号）', { exact: true }).click()
  await dialog.getByText('已选 35 / 47', { exact: true }).waitFor()
  await dialog.getByText('未分组（12 个账号）', { exact: true }).click()
  await dialog.getByText('已选 47 / 47', { exact: true }).waitFor()
  await dialog.getByRole('button', { name: '展开线上账号', exact: true }).click()
  await dialog.getByText('codex-0@example.com', { exact: true }).click()
  await dialog.getByText('已选 46 / 47', { exact: true }).waitFor()
  assert.equal(await dialog.getByRole('checkbox', { name: '线上账号（20 个账号）', exact: true }).evaluate(node => node.indeterminate), true)
  await dialog.getByText('线上账号（20 个账号）', { exact: true }).click()
  await dialog.getByRole('textbox', { name: '搜索分组或账号', exact: true }).fill('codex-46@')
  await dialog.getByRole('checkbox', { name: 'codex-46@example.com', exact: true }).waitFor()
  await dialog.getByRole('textbox', { name: '搜索分组或账号', exact: true }).fill('')
  await dialog.getByRole('button', { name: '收起线上账号', exact: true }).click()
  await dialog.getByRole('button', { name: '清空', exact: true }).click()
  assert.equal(await dialog.getByRole('button', { name: '全选', exact: true }).isEnabled(), true)
  await dialog.getByRole('button', { name: '全选', exact: true }).click()
  await dialog.getByText('已选 47 / 47', { exact: true }).waitFor()
  await dialog.getByText('本轮共 141 个样本', { exact: false }).waitFor()
  await dialog.getByRole('button', { name: '清空', exact: true }).click()
  await dialog.getByText('已选 0 / 47', { exact: true }).waitFor()
  assert.equal(await dialog.getByRole('button', { name: '仅保存', exact: true }).isDisabled(), true)
  await dialog.getByRole('button', { name: '全选', exact: true }).click()
  await dialog.getByLabel('任务名称', { exact: true }).fill('47 个账号巡检')
  async function select(label, option) {
    const trigger = dialog.getByRole('combobox', { name: label, exact: true })
    await trigger.click()
    const controls = await trigger.getAttribute('aria-controls')
    await page.locator(`[id="${controls}"]`).getByRole('option', { name: option, exact: true }).click()
  }
  await select('计量 Key', '模型体检专用')
  await select('测试模型', 'gpt-6-astra')
  await select('预期模型', 'gpt-6-astra')
  assert.equal(await dialog.getByRole('button', { name: '仅保存', exact: true }).isEnabled(), true)
  await page.getByRole('listbox').last().waitFor({ state: 'hidden' })
  await page.screenshot({ path: `${root}/review/screenshots/schedules-47-accounts.png`, fullPage: true, animations: 'disabled' })
  await dialog.getByRole('button', { name: '仅保存', exact: true }).click()
  await dialog.waitFor({ state: 'hidden' })
  assert.equal(saves.length, 1)
  assert.deepEqual(saves[0].account_ids, accounts.map(account => account.id))
  assert.equal(saves[0].enable, false)
  await page.getByRole('row').filter({ hasText: '47 个账号巡检' }).getByRole('button', { name: '编辑', exact: true }).click()
  await dialog.getByText('已选 47 / 47', { exact: true }).waitFor()
  await dialog.getByRole('button', { name: '取消', exact: true }).click()

  await page.getByRole('row').filter({ hasText: '47 个账号巡检' }).getByRole('button', { name: '编辑', exact: true }).click()
  await dialog.getByText('线上账号（20 个账号）', { exact: true }).waitFor()
  await dialog.getByRole('button', { name: '清空', exact: true }).click()
  await dialog.getByText('线上账号（20 个账号）', { exact: true }).click()
  await dialog.getByText('未分组（12 个账号）', { exact: true }).click()
  await dialog.getByRole('button', { name: '仅保存', exact: true }).click()
  await dialog.waitFor({ state: 'hidden' })
  assert.deepEqual(saves.at(-1).account_ids, [...accounts.slice(0,20),...accounts.slice(35)].map(a=>a.id))
  await page.getByRole('row').filter({ hasText: '47 个账号巡检' }).getByRole('button', { name: '编辑', exact: true }).click()
  await dialog.getByText('已选 32 / 47', { exact: true }).waitFor()
  await dialog.getByText('线上账号（20 个账号）', { exact: true }).waitFor()
  assert.equal(await dialog.getByRole('checkbox', { name: '线上账号（20 个账号）', exact: true }).isChecked(), true)
  assert.equal(await dialog.getByRole('checkbox', { name: '未分组（12 个账号）', exact: true }).isChecked(), true)
  await dialog.getByRole('button', { name: '全选', exact: true }).click()
  await dialog.getByRole('button', { name: '仅保存', exact: true }).click()
  await dialog.waitFor({ state: 'hidden' })

  schedules[0].account_ids.push('removed-account')
  await page.reload()
  await page.getByRole('row').filter({ hasText: '47 个账号巡检' }).getByRole('button', { name: '编辑', exact: true }).click()
  await dialog.getByRole('button', { name: '移除不可用账号', exact: true }).click()
  await dialog.getByText('已选 47 / 47', { exact: true }).waitFor()
  await dialog.getByRole('button', { name: '仅保存', exact: true }).click()
  await dialog.waitFor({ state: 'hidden' })
  assert.deepEqual(saves.at(-1).account_ids, accounts.map(account => account.id))

  // A finished round with recycled failed reports must retain errors and unlock editing/deletion.
  const completed = Date.now()
  schedules[0].round = { due_at: completed - 60000, completed_at: completed, members: accounts.map((account, index) => ({ account_id: account.id, run_id: `run-${index}`, finished_at: completed, error: index < 20 ? `账号 ${index} 授权失败，本轮已跳过` : null, run: null })) }
  await page.reload()
  const row = page.getByRole('row').filter({ hasText: '47 个账号巡检' })
  await row.getByText('已结束 47 / 47 个账号', { exact: true }).waitFor()
  await row.getByText('部分账号检测失败或已跳过，请查看本轮', { exact: true }).waitFor()
  assert.equal(await row.getByRole('button', { name: '编辑', exact: true }).isEnabled(), true)
  assert.equal(await row.getByRole('button', { name: '删除', exact: true }).isEnabled(), true)
  await row.getByRole('button', { name: '查看本轮', exact: true }).click()
  await dialog.getByText('账号 0 授权失败，本轮已跳过', { exact: true }).waitFor()
  await page.keyboard.press('Escape')
  await dialog.waitFor({ state: 'hidden' })
  await row.getByRole('button', { name: '删除', exact: true }).click()
  await dialog.getByRole('button', { name: '删除', exact: true }).click()
  await row.waitFor({ state: 'hidden' })
  assert.equal(schedules.length, 0)

  // The denominator follows the returned directory rather than a fixed 16 or 47.
  for (const count of [48, 0]) {
    accounts = Array.from({ length: count }, (_, i) => ({ id: `account-${i}`, name: `Codex ${i}`, email: null, plan: 'pro' }))
    schedules = []
    await page.reload()
    await page.getByRole('button', { name: '新建定时任务', exact: true }).click()
    await dialog.getByText(`已选 0 / ${count}`, { exact: true }).first().waitFor()
    await dialog.getByRole('checkbox', { name: `未分组（${Math.max(0,count-35)} 个账号）`, exact: true }).waitFor()
    assert.equal(await dialog.getByRole('button', { name: '全选', exact: true }).isEnabled(), count > 0)
    if (count) {
      await dialog.getByRole('button', { name: '全选', exact: true }).click()
      await dialog.getByText(`已选 ${count} / ${count}`, { exact: true }).waitFor()
    }
    await dialog.getByRole('button', { name: '取消', exact: true }).click()
  }
  failGroups = true
  await page.getByRole('button', { name: '新建定时任务', exact: true }).click()
  await dialog.getByText('模拟分组读取失败', { exact: false }).waitFor()
  assert.equal(await dialog.getByRole('button', { name: '仅保存', exact: true }).isDisabled(), true)
  failGroups = false
  await dialog.getByRole('button', { name: '重试', exact: true }).click()
  await dialog.getByRole('checkbox', { name: '未分组（0 个账号）', exact: true }).waitFor()
  await dialog.getByRole('button', { name: '取消', exact: true }).click()
  assert.deepEqual(errors, [])
  console.log('Schedules passed: group/ungrouped selection, overlapping groups, partial selection, search, save/reopen, error retry, and edit/delete after completion.')
} finally {
  await browser.close()
}
