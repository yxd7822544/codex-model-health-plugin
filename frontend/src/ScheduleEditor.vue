<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import { ZDialog, ZInput, ZInputNumber, ZSelect, ZSegmented, ZButton } from '@codex-proxy/ui'
import { api } from './api'
import type { Bootstrap } from './api'
import { schedulesApi } from './api/schedules'
import type { Schedule, ScheduleDraft } from './api/schedules'
import { accountDisplayName } from './accountDisplay'

const props = defineProps<{bootstrap: Bootstrap; schedule: Schedule | null}>()
const open = defineModel<boolean>({required:true})
const emit = defineEmits<{saved: []}>()
const draft = ref<ScheduleDraft>(empty())
const pending = shallowRef<'save' | 'enable' | null>(null), error = shallowRef(''), modelsLoading = shallowRef(false)
const saving = computed(()=>pending.value !== null)
const models = shallowRef<string[]>([])
const catalog = computed(() => props.bootstrap.catalog.methods.find(m => m.method === draft.value.detection.method))
const accounts = computed(() => props.bootstrap.accounts.map(a=>({value:a.id,label:accountDisplayName(a)})))
const keys = computed(() => props.bootstrap.keys.map(k=>({value:k.id,label:k.name})))
const methods = computed(() => props.bootstrap.catalog.methods.map(m=>({value:m.method,label:m.label})))
const candidates = computed(() => catalog.value?.models.map(m=>({value:m.id,label:m.label || m.id})) ?? [])
const tiers = computed(() => catalog.value?.tiers.map(t=>({value:t.id,label:`${t.count} 题`})) ?? [])
const sampleCount = computed(() => catalog.value?.tiers.find(t=>t.id === draft.value.detection.tier)?.count ?? 0)
const valid = computed(() => draft.value.name.trim() && draft.value.account_ids.length > 0 && draft.value.account_ids.every(id=>accounts.value.some(a=>a.value===id)) && draft.value.detection.key_id && models.value.includes(draft.value.detection.request_model) && draft.value.expected_model && Number.isInteger(draft.value.interval_minutes) && draft.value.interval_minutes >= 1 && draft.value.interval_minutes <= 43200 && !modelsLoading.value)
function empty(): ScheduleDraft {return {name:'',interval_minutes:360,expected_model:'',account_ids:[],detection:{background:true,method:'modeltrace',account_id:'',key_id:'',request_model:'',claimed_model:'unified',tier:'fixed'}}}
watch(open, value => {
  if (!value) return
  error.value = ''
  const item = props.schedule
  draft.value = item ? {id:item.id,revision:item.revision,name:item.name,interval_minutes:item.interval_minutes,expected_model:item.expected_model,account_ids:[...item.account_ids],detection:{...item.detection}} : empty()
})
watch(() => [open.value, draft.value.detection.key_id] as const, async ([visible,key], _, cleanup) => {
  let current = true
  cleanup(()=>{current=false})
  models.value = []
  modelsLoading.value = false
  if (!visible || !key) return
  modelsLoading.value = true
  try {const result = await api<{models:string[]}>('models',{key_id:key}); if (current) models.value=result.models}
  catch(cause) {if(current) error.value=cause instanceof Error ? cause.message : String(cause)}
  finally {if(current) modelsLoading.value=false}
})
function changeMethod() {
  draft.value.expected_model = ''
  draft.value.detection.tier = draft.value.detection.method === 'modeltrace' ? 'fixed' : 'low'
}
async function save(enable: boolean) {
  if (!valid.value || saving.value) return
  pending.value = enable ? 'enable' : 'save'; error.value = ''
  try {
    const value = draft.value
    await schedulesApi.save({...value,name:value.name.trim(),detection:{...value.detection,account_id:value.account_ids[0]!,background:true,claimed_model:value.detection.method === 'modeltrace' ? 'unified' : value.expected_model}}, enable)
    open.value = false; emit('saved')
  } catch(cause) {error.value = cause instanceof Error ? cause.message : String(cause)}
  finally {pending.value=null}
}
</script>

<template>
  <ZDialog v-model="open" :title="schedule ? '编辑定时任务' : '新建定时任务'" width="40rem" :close-on-click-modal="!saving" :show-close="!saving" :close-on-press-escape="!saving">
    <div class="grid gap-4">
      <p v-if="error" role="alert" class="sample-error m-0">{{ error }}</p>
      <div class="form-grid">
        <div class="field col-span-full"><label for="schedule-name">任务名称</label><ZInput id="schedule-name" v-model="draft.name" :maxlength="80" :disabled="saving" placeholder="例如 主账号模型巡检" /></div>
        <div class="field col-span-full"><label id="schedule-method">检测方法</label><ZSegmented v-model="draft.detection.method" :options="methods" aria-labelledby="schedule-method" :disabled="saving" @update:model-value="changeMethod" /></div>
        <div class="field col-span-full"><div class="flex flex-wrap items-center justify-between gap-2"><label id="schedule-account">检测账号（可多选）</label><div class="flex items-center gap-2"><span class="text-z-xs text-z-text-secondary">已选 {{ draft.account_ids.length }} / {{ accounts.length }}</span><ZButton size="small" :disabled="saving || !accounts.length" @click="draft.account_ids=accounts.map(a=>a.value)">全选</ZButton><ZButton size="small" :disabled="saving || !draft.account_ids.length" @click="draft.account_ids=[]">清空</ZButton></div></div><ZSelect v-model="draft.account_ids" :options="accounts" aria-labelledby="schedule-account" :disabled="saving" placeholder="搜索并选择检测账号" multiple filterable collapse-tags collapse-tags-tooltip :max-collapse-tags="3" /></div>
        <div class="field"><label id="schedule-key">计量 Key</label><ZSelect v-model="draft.detection.key_id" :options="keys" aria-labelledby="schedule-key" :disabled="saving" placeholder="选择计量 Key" filterable /></div>
        <div class="field"><label id="schedule-model">测试模型</label><ZSelect v-model="draft.detection.request_model" :options="models.map(m=>({value:m,label:m}))" aria-labelledby="schedule-model" :disabled="saving || !draft.detection.key_id" :loading="modelsLoading" placeholder="先选择计量 Key" filterable /></div>
        <div class="field"><label id="schedule-expected">预期模型</label><ZSelect v-model="draft.expected_model" :options="candidates" aria-labelledby="schedule-expected" :disabled="saving" placeholder="选择报告应匹配的模型" filterable /></div>
        <div class="field"><label for="schedule-interval">执行间隔（分钟）</label><ZInputNumber id="schedule-interval" aria-label="执行间隔（分钟）" v-model="draft.interval_minutes" :min="1" :max="43200" :precision="0" :disabled="saving" /></div>
        <div v-if="draft.detection.method === 'meow'" class="field"><label id="schedule-tier">检测档位</label><ZSelect v-model="draft.detection.tier" :options="tiers" aria-labelledby="schedule-tier" :disabled="saving" /></div>
      </div>
      <p class="m-0 text-z-sm text-z-text-secondary">每账号 {{ sampleCount }} 个样本，本轮共 {{ sampleCount * draft.account_ids.length }} 个样本，真实调用计入所选 Key 用量<br>启用后立即检查首轮，全部账号结束后再计时；仅保存则保持停用</p>
      <p class="m-0 text-z-xs text-z-text-secondary">{{ draft.detection.method === 'modeltrace' ? 'ModelTrace 第一候选与预期模型不同时告警' : 'Meow 使用预期模型作为对照基线，行为偏移时告警' }}<br>模型别名可选择对应的预期模型，证据不足不发送不匹配告警</p>
      <div class="flex flex-wrap justify-end gap-2"><ZButton :disabled="saving" @click="open=false">取消</ZButton><ZButton :disabled="!valid || saving" :loading="pending === 'save'" @click="save(false)">仅保存</ZButton><ZButton type="primary" :disabled="!valid || saving" :loading="pending === 'enable'" @click="save(true)">保存并启用</ZButton></div>
    </div>
  </ZDialog>
</template>
