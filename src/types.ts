export interface Task {
  id: string;
  summary: string;
  description: string;
  /** "YYYY-MM-DD"；null = 收集箱 */
  day: string | null;
  remind_at: number | null;
  done: boolean;
  completed_at: number | null;
  position: number;
  parent_id: string | null;
  created_at: number;
  updated_at: number;
  lark_guid: string | null;
  lark_url: string | null;
  /** 两侧都改了同一字段，已保留本地值 */
  conflict: boolean;
}

export interface NewTask {
  summary: string;
  description?: string;
  day?: string | null;
  parent_id?: string | null;
}

/** 省略字段 = 不改动；显式传 null = 清空 */
export interface TaskPatch {
  summary?: string;
  description?: string;
  day?: string | null;
  remind_at?: number | null;
  done?: boolean;
}

export type SyncState =
  | "idle"
  | "syncing"
  | "offline"
  | "unauthorized"
  | "disabled"
  | "error";

export interface SyncStatus {
  state: SyncState;
  last_sync_at: number | null;
  pending: number;
  message: string | null;
}

export interface LarkAccount {
  name: string | null;
  open_id: string | null;
  ready: boolean;
}

/** [日期, 未完成数, 总数] */
export type DayCount = [string, number, number];

/** 浮窗吸附到屏幕的哪一边 */
export type Side = "left" | "right";
