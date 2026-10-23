import { reactive, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { SyncStatus, Task } from "./types";
import { addDays, todayStr } from "./date";

/** 当前选中的日期；null 表示收集箱。下方任务列表跟着它。 */
export const view = reactive({
  day: todayStr() as string | null,
  /** 7 天条最左边那天。只管滚动，跟选中日互不干扰。 */
  anchor: todayStr(),
  /** day | overview */
  mode: "day" as "day" | "overview",
});

export const tasks = ref<Task[]>([]);
export const subtasks = reactive<Record<string, Task[]>>({});
export const expanded = reactive<Record<string, boolean>>({});
export const loading = ref(false);

/** 正在输入新待办时贴边窗口不要收起来 */
export const holdOpen = ref(false);

/** 设置面板是否打开 */
export const settingsOpen = ref(false);

export const sync = ref<SyncStatus>({
  state: "idle",
  last_sync_at: null,
  pending: 0,
  message: null,
});

/** 缺失的飞书写权限；非空时 UI 提示用户去授权 */
export const missingScopes = ref<string[]>([]);

/** 选中某天并加载它的任务；选中日不在 7 天窗口里就把窗口挪过去 */
export async function selectDay(day: string | null) {
  view.day = day;
  if (day && (day < view.anchor || day > addDays(view.anchor, 6))) {
    view.anchor = day;
  }
  await reload();
}

/** 只滚动 7 天条，不改选中日 */
export function scrollDays(n: number) {
  view.anchor = addDays(view.anchor, n);
}

export async function reload() {
  loading.value = true;
  try {
    tasks.value = await api.listTasks(view.day);
    // 只刷新已展开的子待办，避免无谓的往返
    for (const id of Object.keys(expanded)) {
      if (expanded[id]) subtasks[id] = await api.listSubtasks(id);
    }
  } finally {
    loading.value = false;
  }
}

export async function toggleExpand(t: Task) {
  expanded[t.id] = !expanded[t.id];
  if (expanded[t.id] && !subtasks[t.id]) {
    subtasks[t.id] = await api.listSubtasks(t.id);
  }
}

export async function add(summary: string) {
  const text = summary.trim();
  if (!text) return;
  await api.createTask({ summary: text, day: view.day });
  await reload();
}

/** 拖拽结束后一小段时间内忽略标题点击，避免松手误进编辑 */
export const dragUi = { ignoreClickUntil: 0, draggingId: "" };

export function noteTaskDrag() {
  dragUi.ignoreClickUntil = Date.now() + 250;
  dragUi.draggingId = "";
}

/** 按给定 id 顺序重排当前视图里的一组任务（未完成或已完成各自独立） */
export async function reorder(ids: string[]) {
  if (ids.length < 2) return;
  const set = new Set(ids);
  const moved = ids
    .map((id) => tasks.value.find((t) => t.id === id))
    .filter((t): t is Task => !!t);
  if (moved.length < 2) return;
  const others = tasks.value.filter((t) => !set.has(t.id));
  tasks.value = moved[0].done ? [...others, ...moved] : [...moved, ...others];
  await api.reorderTasks(view.day, ids);
}

export async function addSub(parent: Task, summary: string) {
  const text = summary.trim();
  if (!text) return;
  await api.createTask({ summary: text, day: parent.day, parent_id: parent.id });
  subtasks[parent.id] = await api.listSubtasks(parent.id);
}

export async function toggleDone(t: Task) {
  await api.updateTask(t.id, { done: !t.done });
  await reload();
}

export async function rename(t: Task, summary: string) {
  const text = summary.trim();
  if (!text || text === t.summary) return;
  await api.updateTask(t.id, { summary: text });
  await reload();
}

/** 移动到某天；传 null 丢回收集箱 */
export async function moveTo(t: Task, day: string | null) {
  await api.updateTask(t.id, { day });
  await reload();
}

export async function remove(t: Task) {
  await api.deleteTask(t.id);
  delete subtasks[t.id];
  delete expanded[t.id];
  await reload();
}

export async function refreshSync() {
  sync.value = await api.syncStatus();
}

/** 打开面板时拉飞书；冷却内后端会直接跳过 */
export function pullOnView() {
  void api.pullOnView();
}

export async function checkScopes() {
  try {
    missingScopes.value = await api.larkCheck();
  } catch {
    // lark-cli 没装或没登录，同步状态那边会显示，这里不打扰用户
    missingScopes.value = [];
  }
}

/** 打开浏览器补写权限；返回仍缺失的 scope */
export async function grantWriteScopes() {
  missingScopes.value = await api.larkGrantWrite();
  await refreshSync();
  return missingScopes.value;
}

/** 后端推变更时刷新，避免前端轮询 */
export async function initListeners() {
  await listen("tasks-changed", () => reload());
  await listen<SyncStatus>("sync-status", (e) => {
    const prev = sync.value.state;
    sync.value = e.payload;
    // 同步成功后重新查权限，授权条才能消失
    if (e.payload.state === "idle" && prev !== "idle") {
      void checkScopes();
    }
  });
}
