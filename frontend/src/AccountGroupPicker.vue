<script setup lang="ts">
import { computed, ref } from 'vue'
import { ZButton, ZCheckbox, ZInput } from '@codex-proxy/ui'
import type { Bootstrap } from './api'
import type { AccountGroup } from './api/accountGroups'
import { accountDisplayName } from './accountDisplay'

const props = defineProps<{accounts: Bootstrap['accounts']; groups: AccountGroup[]; disabled?: boolean}>()
const selected = defineModel<string[]>({required: true})
const search = ref(''), expanded = ref(new Set<string>())
const selectedIds = computed(() => new Set(selected.value))
const unavailable = computed(() => selected.value.filter(id => !props.accounts.some(account => account.id === id)))
const groups = computed(() => {
  const accounts = new Map(props.accounts.map(account => [account.id, account]))
  const query = search.value.trim().toLocaleLowerCase()
  return props.groups.map(group => {
    const members = [...new Set(group.account_ids)].flatMap(id => accounts.has(id) ? [accounts.get(id)!] : [])
    const visible = !query || group.name.toLocaleLowerCase().includes(query) ? members : members.filter(account => `${accountDisplayName(account)} ${account.name}`.toLocaleLowerCase().includes(query))
    return {...group, members, visible, selectedCount: members.filter(account => selectedIds.value.has(account.id)).length}
  }).sort((a, b) => Number(!a.members.length) - Number(!b.members.length)).filter(group => !query || group.visible.length || group.name.toLocaleLowerCase().includes(query))
})
function selectAccounts(ids: string[], checked: boolean) {
  const next = new Set(selected.value)
  for (const id of ids) {if (checked) next.add(id); else next.delete(id)}
  selected.value = [...next]
}
function toggleGroup(id: string) {
  if (expanded.value.has(id)) expanded.value.delete(id)
  else expanded.value.add(id)
}
</script>

<template>
  <div class="grid gap-2">
    <p v-if="unavailable.length" class="m-0 text-z-xs text-z-warning-text">有 {{ unavailable.length }} 个已选账号当前不可用 <ZButton size="small" :disabled="disabled" @click="selectAccounts(unavailable, false)">移除不可用账号</ZButton></p>
    <ZInput v-model="search" aria-label="搜索分组或账号" placeholder="搜索分组或账号" :disabled="disabled" />
    <div class="max-h-60 overflow-y-auto rounded-z-sm border border-z-border" aria-label="账号分组">
      <div v-for="group in groups" :key="group.id" class="border-b border-z-border p-3 last:border-b-0">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <ZCheckbox :label="`${group.name}（${group.members.length} 个账号）`" :model-value="group.members.length > 0 && group.selectedCount === group.members.length" :indeterminate="group.selectedCount > 0 && group.selectedCount < group.members.length" :disabled="disabled || !group.members.length" @update:model-value="value => selectAccounts(group.members.map(account => account.id), value)" />
          <div class="flex items-center gap-2 text-z-xs text-z-text-secondary"><span v-if="!group.enabled">分组已停用</span><span>已选 {{ group.selectedCount }} / {{ group.members.length }}</span><ZButton size="small" :disabled="disabled || !group.members.length" :aria-label="`${expanded.has(group.id) ? '收起' : '展开'}${group.name}`" :aria-expanded="expanded.has(group.id) || !!search.trim()" @click="toggleGroup(group.id)">{{ expanded.has(group.id) ? '收起' : '展开' }}</ZButton></div>
        </div>
        <div v-if="expanded.has(group.id) || search.trim()" class="mt-2 grid gap-2 pl-4">
          <ZCheckbox v-for="account in group.visible" :key="account.id" :label="accountDisplayName(account)" :model-value="selectedIds.has(account.id)" :disabled="disabled" @update:model-value="value => selectAccounts([account.id], value)" />
        </div>
      </div>
      <p v-if="!groups.length" class="m-0 p-3 text-z-sm text-z-text-secondary">没有匹配的分组或账号</p>
    </div>
    <span class="text-z-xs text-z-text-secondary">勾选分组可选中组内全部账号，也可展开单独调整；多个分组中的相同账号只检测一次。</span>
  </div>
</template>
