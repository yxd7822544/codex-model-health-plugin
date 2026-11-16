import { api } from './index'
import type { AlertDelivery, Method, Report, Status } from './index'

export interface Detection { background: boolean; method: Method; account_id: string; key_id: string; request_model: string; claimed_model: string; tier: string }
export interface ScheduleDraft { id?: string; revision?: number; name: string; interval_minutes: number; expected_model: string; account_ids: string[]; detection: Detection }
export interface ScheduledReport { id: string; status: Status; inflight: string | null; background?: boolean; queued_at?: number | null; report: Report; task_error: string | null; alert: AlertDelivery | null }
export interface RoundMember {account_id: string; run_id: string | null; finished_at: number | null; error: string | null; run: ScheduledReport | null}
export interface Schedule extends ScheduleDraft {
  id: string; revision: number; enabled: boolean; next_run_at: number | null; last_run_id: string | null; error: string | null
  last_run: ScheduledReport | null
  round: null | {due_at: number; completed_at: number | null; members: RoundMember[]}
}
export const schedulesApi = {
  list: () => api<{schedules: Schedule[]; alerts_configured: boolean}>('schedules'),
  save: (value: ScheduleDraft, enable = false) => api('schedules/save', {...value, enable}),
  control: (schedule: Schedule, action: string) => api('schedules/control', {id:schedule.id, revision:schedule.revision, action}),
}
