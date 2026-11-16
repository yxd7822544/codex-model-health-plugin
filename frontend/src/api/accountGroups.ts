import { api } from './index'

export interface AccountGroup { id: string; name: string; enabled: boolean; account_ids: string[] }
export const loadAccountGroups = () => api<{groups: AccountGroup[]}>('account-groups')
