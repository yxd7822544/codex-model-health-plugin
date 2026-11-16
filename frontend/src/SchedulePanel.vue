<script setup lang="ts">
import { computed, onMounted, onScopeDispose, shallowRef } from 'vue'
import { ZButton, ZCard, ZTable, ZTag, ZDialog } from '@codex-proxy/ui'
import type { TableColumn } from '@codex-proxy/ui'
import type { Bootstrap } from './api'
import { api } from './api'
import { schedulesApi } from './api/schedules'
import type { Schedule, RoundMember } from './api/schedules'
import { alertStatusText, remediationStatusText, canRetryRemediation, formatTime, methodText, verdict, runStatusText } from './presentation'
import { accountDisplayName } from './accountDisplay'
import ScheduleEditor from './ScheduleEditor.vue'

const props = defineProps<{bootstrap: Bootstrap}>()
const emit = defineEmits<{openRun: [id: string]; showHistory: [schedule: {id: string; name: string}]}>()
const items = shallowRef<Schedule[]>([]), configured = shallowRef<boolean | null>(null), loading = shallowRef(true), busy = shallowRef(''), error = shallowRef('')
const editor = shallowRef(false), editing = shallowRef<Schedule | null>(null), deleting = shallowRef<Schedule | null>(null)
const remediationConfigured=shallowRef(false)
const detailId = shallowRef('')
const detail = computed(()=>items.value.find(item=>item.id===detailId.value) ?? null)
let disposed=false, sequence=0, timer: ReturnType<typeof setTimeout> | undefined
const columns: TableColumn<Schedule>[] = [
  {key:'name',label:'定时任务',kind:'custom',size:'xl'},
  {key:'enabled',label:'状态',kind:'custom',size:'lg'},
  {key:'next_run_at',label:'执行计划',kind:'custom',size:'xl'},
  {key:'result',label:'最近检测 / 告警',kind:'custom',size:'xl'},
  {key:'actions',label:'操作',kind:'actions',size:'2xl'},
]
const memberColumns: TableColumn<RoundMember>[] = [
  {key:'account_id',label:'账号',kind:'custom',size:'xl'},
  {key:'result',label:'检测状态',kind:'custom',size:'xl'},
  {key:'alert',label:'飞书告警',kind:'custom',size:'lg'},
  {key:'remediation',label:'账号分组处理',kind:'custom',size:'xl'},
  {key:'actions',label:'操作',kind:'actions',size:'md'},
]
function accountName(id: string) {const account = props.bootstrap.accounts.find(a=>a.id === id); return account ? accountDisplayName(account) : id}
function unfinished(item: Schedule) {return item.round ? item.round.completed_at === null : !!item.last_run && (!['completed','stopped'].includes(item.last_run.status) || !!item.last_run.inflight)}
function paused(item: Schedule) {return item.round ? item.round.members.some(m=>m.run?.status==='paused') : item.last_run?.status==='paused'}
function state(item: Schedule) {return !item.enabled ? '已停用' : paused(item) ? '跳过暂停账号中' : unfinished(item) ? '本轮执行中' : '已启用'}
function ended(item: Schedule) {return item.round?.members.filter(m=>m.finished_at!==null || (m.run && ['completed','stopped'].includes(m.run.status) && !m.run.inflight)).length ?? 0}
function openReport(id: string) {detailId.value=''; emit('openRun',id)}
async function load() {
  const request=++sequence
  try {
    const data=await schedulesApi.list()
    if (!disposed && request===sequence) {items.value=data.schedules; configured.value=data.alerts_configured; remediationConfigured.value=data.remediation_configured; error.value=''}
  } catch(cause) {if(!disposed && request===sequence) error.value=cause instanceof Error ? cause.message : String(cause)}
  finally {if(!disposed && request===sequence) loading.value=false}
}
async function poll() {
  if (disposed) return
  if (!busy.value && document.visibilityState === 'visible') await load()
  if(!disposed) timer=setTimeout(poll,5000)
}
function edit(item: Schedule | null) {editing.value=item; editor.value=true}
async function control(item: Schedule, action: string) {
  if(busy.value) return
  ++sequence; busy.value=item.id; error.value=''
  try {await schedulesApi.control(item,action); deleting.value=null; await load()}
  catch(cause) {error.value=cause instanceof Error ? cause.message : String(cause)}
  finally {busy.value=''}
}
async function retryRemediation(id:string) {
  if(busy.value) return
  ++sequence;busy.value=`remediation:${id}`;error.value=''
  try {await api('runs/remediation/retry',{id});await load()}
  catch(cause) {error.value=cause instanceof Error ? cause.message : String(cause)}
  finally {busy.value=''}
}
onMounted(()=>{void poll()})
onScopeDispose(()=>{disposed=true; ++sequence; clearTimeout(timer)})
</script>

<template>
  <ZCard>
    <template #header><div class="flex flex-wrap items-center justify-between gap-3"><div><h2>定时任务</h2><p class="subtitle">按设定间隔检测并保存结果，飞书告警可选</p></div><ZButton type="primary" :disabled="loading || items.length >= 16" @click="edit(null)">新建定时任务</ZButton></div></template>
    <template #body>
      <p v-if="error" class="sample-error" role="alert">{{ error }} <ZButton size="small" @click="load">刷新状态</ZButton></p>
      <p v-if="!loading && configured !== null" class="m-0 mb-4 text-z-sm text-z-text-secondary"><ZTag :type="configured ? 'success' : 'default'">{{ configured ? '飞书已配置' : '飞书未配置' }}</ZTag><span class="ml-2">{{ configured ? '停用仅停止后续调度，本轮可在检测报告中结束' : '不影响检测和历史记录；需要消息告警时，到插件设置填写 Webhook' }}</span></p>
      <p v-if="!loading" class="m-0 mb-4 text-z-sm text-z-text-secondary">{{ remediationConfigured ? '不匹配账号将移出原分组，移入 GPT降智分组，并从本计划后续检测名单移除。' : '自动改组未配置：在插件设置中填写宿主管理 API Key 后生效。' }}</p>
      <ZTable :columns="columns" :data="items" :loading="loading" empty-text="还没有定时任务，创建后可单独启用或停用">
        <template #name="{row}"><div class="grid gap-1"><strong class="font-emphasis">{{ row.name }}</strong><span class="text-z-xs text-z-text-secondary">后续检测 {{ row.account_ids.length }} 个账号</span><span class="text-z-xs text-z-text-secondary">{{ methodText(row.detection.method) }} · {{ row.detection.request_model }}</span><span class="text-z-xs text-z-text-secondary">预期 {{ row.expected_model }}</span></div></template>
        <template #enabled="{row}"><ZTag :type="!row.enabled ? 'default' : paused(row) ? 'warning' : 'success'">{{ state(row) }}</ZTag><p v-if="row.error" class="text-z-xs text-z-warning-text">{{ row.error }}</p></template>
        <template #next_run_at="{row}"><div class="grid gap-1"><span>每 {{ row.interval_minutes }} 分钟</span><span class="text-z-xs text-z-text-secondary">{{ unfinished(row) ? row.enabled ? '全部账号结束后开始计时' : '本轮继续，结束后不再调度' : !row.enabled ? '启用后立即检查首轮' : `下次 ${formatTime(row.next_run_at ?? 0)}` }}</span></div></template>
        <template #result="{row}"><div v-if="row.round" class="grid gap-1"><span>已结束 {{ ended(row) }} / {{ row.round.members.length }} 个账号</span><span v-if="row.round.members.some(m=>m.error)" class="text-z-xs text-z-warning-text">部分账号检测失败或已跳过，请查看本轮</span><span v-if="row.round.members.some(m=>m.run?.alert?.status==='failed')" class="text-z-xs text-z-warning-text">部分飞书告警发送失败</span><span v-if="paused(row)" class="text-z-xs text-z-warning-text">暂停账号将结束本轮尝试，随后继续其他账号</span></div><div v-else-if="row.last_run" class="grid gap-1"><span>{{ verdict(row.last_run.report,row.last_run.status) }}</span></div><span v-else class="text-z-text-secondary">{{ row.last_run_id ? '检测记录已过保留期' : '尚未执行' }}</span></template>
        <template #actions="{row}"><div class="flex flex-wrap gap-2"><ZButton size="small" :disabled="!!busy || (!row.enabled && !row.account_ids.length)" :loading="busy === row.id" @click="control(row,row.enabled ? 'disable' : 'enable')">{{ row.enabled ? '停用' : '启用' }}</ZButton><ZButton v-if="row.enabled" size="small" :disabled="!!busy || unfinished(row)" @click="control(row,'run_now')">立即检测</ZButton><ZButton v-else size="small" :disabled="!!busy || unfinished(row)" :title="unfinished(row) ? '本轮结束后可编辑' : undefined" @click="edit(row)">编辑</ZButton><ZButton v-if="row.round" size="small" @click="detailId=row.id">查看本轮</ZButton><ZButton v-else-if="row.last_run" size="small" @click="openReport(row.last_run.id)">报告</ZButton><ZButton size="small" @click="emit('showHistory',{id:row.id,name:row.name})">执行历史</ZButton><ZButton v-if="!row.enabled" size="small" :disabled="!!busy || unfinished(row)" :title="unfinished(row) ? '本轮结束后可删除' : undefined" @click="deleting=row">删除</ZButton></div></template>
      </ZTable>
    </template>
  </ZCard>
  <ScheduleEditor v-model="editor" :bootstrap="bootstrap" :schedule="editing" @saved="load" />
  <ZDialog :model-value="!!detail" :title="`${detail?.name ?? ''} · 本轮检测`" width="60rem" @update:model-value="value=>{if(!value) detailId=''}"><p class="text-z-sm text-z-text-secondary">{{ detail && unfinished(detail) ? '各账号依次执行；失败或暂停账号保留原因后跳过，不阻塞后续账号' : '本轮已结束，下一轮开始时更新此列表' }}。完整历史至少保留 1000 条，可在执行历史中翻页查看</p><ZTable :columns="memberColumns" :data="detail?.round?.members ?? []"><template #account_id="{row}">{{ accountName(row.account_id) }}</template><template #result="{row}"><p v-if="row.error" class="text-z-warning-text">{{ row.error }}</p><div v-else-if="row.run"><span>{{ runStatusText(row.run) }}</span><p v-if="row.run.status === 'completed'" class="text-z-xs text-z-text-secondary">{{ verdict(row.run.report,row.run.status) }}</p><p v-if="row.run.task_error" class="text-z-xs text-z-warning-text">{{ row.run.task_error }}</p></div><span v-else>{{ row.finished_at !== null ? '已结束 · 报告已过保留期' : '等待入队' }}</span></template><template #alert="{row}"><template v-if="row.run?.alert"><span>{{ ['completed','stopped'].includes(row.run.status) ? alertStatusText[row.run.alert.status] : '检测完成后判定' }}</span><p v-if="row.run.alert.error" class="text-z-xs text-z-warning-text">{{ row.run.alert.error }}</p></template><span v-else>—</span></template><template #remediation="{row}"><div v-if="row.run?.scheduled?.remediation && ['completed','stopped'].includes(row.run.status)"><span>{{ remediationStatusText[row.run.scheduled.remediation.status] }}</span><p v-if="row.run.scheduled.remediation.error" class="text-z-xs text-z-warning-text">{{ row.run.scheduled.remediation.error }}</p></div><span v-else>—</span></template><template #actions="{row}"><ZButton v-if="row.run" size="small" @click="openReport(row.run.id)">查看报告</ZButton><ZButton v-if="row.run && ['completed','stopped'].includes(row.run.status) && canRetryRemediation(row.run.scheduled?.remediation)" size="small" :disabled="!!busy" :loading="busy===`remediation:${row.run.id}`" @click="retryRemediation(row.run.id)">重试改组</ZButton></template></ZTable></ZDialog>
  <ZDialog :model-value="!!deleting" title="删除定时任务" width="28rem" :show-close="!busy" @update:model-value="value => {if (!value && !busy) deleting=null}"><p>删除「{{ deleting?.name }}」的执行计划，已有检测记录仍保留</p><div class="flex justify-end gap-2"><ZButton :disabled="!!busy" @click="deleting=null">取消</ZButton><ZButton type="danger" :loading="!!busy" @click="deleting && control(deleting,'delete')">删除</ZButton></div></ZDialog>
</template>
