import { invoke } from "@tauri-apps/api/core";
import type {
  DayCount,
  LarkAccount,
  NewTask,
  Side,
  SyncStatus,
  Task,
  TaskPatch,
} from "./types";

export const api = {
  listTasks: (day: string | null) => invoke<Task[]>("list_tasks", { day }),

  listSubtasks: (parentId: string) =>
    invoke<Task[]>("list_subtasks", { parentId }),

  dayCounts: (from: string, to: string) =>
    invoke<DayCount[]>("day_counts", { from, to }),

  createTask: (nt: NewTask) => invoke<Task>("create_task", { new: nt }),

  updateTask: (id: string, patch: TaskPatch) =>
    invoke<Task | null>("update_task", { id, patch }),

  deleteTask: (id: string) => invoke<void>("delete_task", { id }),

  reorderTasks: (day: string | null, ids: string[]) =>
    invoke<void>("reorder_tasks", { day, ids }),

  resolveConflict: (id: string) => invoke<void>("resolve_conflict", { id }),

  syncStatus: () => invoke<SyncStatus>("sync_status"),

  syncNow: () => invoke<number>("sync_now"),

  /** 打开/聚焦面板时拉飞书；冷却内重复调用会被后端跳过 */
  pullOnView: () => invoke<number>("pull_on_view"),

  setSyncEnabled: (on: boolean) => invoke<void>("set_sync_enabled", { on }),

  /** 返回缺失的飞书 scope，空数组表示权限齐了 */
  larkCheck: () => invoke<string[]>("lark_check"),

  larkAccount: () => invoke<LarkAccount>("lark_account"),

  /** 打开浏览器补写权限，等用户扫码完成后返回仍缺失的 scope */
  larkGrantWrite: () => invoke<string[]>("lark_grant_write"),

  // ---- 浮窗贴边吸附 ----

  /** 拖动结束后判断是否吸附，返回吸附到哪边 */
  snapCheck: () => invoke<Side | null>("snap_check"),

  /** 直接吸附到当前屏幕指定一侧，不要求先把窗口拖到边缘 */
  snapTo: (side: Side) => invoke<void>("snap_to", { side }),

  snapUnsnap: () => invoke<void>("snap_unsnap"),

  snapCollapse: (side: Side) => invoke<void>("snap_collapse", { side }),

  snapExpand: (side: Side) => invoke<void>("snap_expand", { side }),

  /** 读回上次的吸附边 */
  snapRestore: () => invoke<Side | null>("snap_restore"),

  /** 光标是否还在窗口外框上（含标题栏、缩放边） */
  snapPointerOver: () => invoke<boolean>("snap_pointer_over"),

  /** 把前端日志写进 snap 日志（设 LILO_SNAP_DEBUG=1 时生效） */
  snapDebug: (msg: string) => invoke<void>("snap_debug", { msg }),

  toggleMini: () => invoke<void>("toggle_mini"),

  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (on: boolean) => invoke<void>("set_autostart", { on }),

  getPanelOpacity: () => invoke<number>("get_panel_opacity"),
  setPanelOpacity: (percent: number) =>
    invoke<number>("set_panel_opacity", { percent }),
};
