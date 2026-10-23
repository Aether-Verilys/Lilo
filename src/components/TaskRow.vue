<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import type { Task } from "../types";
import { timeStr } from "../date";
import {
  addSub,
  expanded,
  dragUi,
  noteTaskDrag,
  remove,
  rename,
  subtasks,
  toggleDone,
  toggleExpand,
} from "../store";

const props = defineProps<{
  task: Task;
  sub?: boolean;
  drop?: "before" | "after" | null;
}>();

const editing = ref(false);
const draft = ref("");
const titleEl = ref<HTMLInputElement | null>(null);
const addingSub = ref(false);
const subDraft = ref("");

async function startEdit() {
  if (Date.now() < dragUi.ignoreClickUntil) return;
  editing.value = true;
  draft.value = props.task.summary;
  await nextTick();
  titleEl.value?.focus();
  titleEl.value?.select();
}

async function commitEdit() {
  if (!editing.value) return;
  editing.value = false;
  await rename(props.task, draft.value);
}

async function commitSub() {
  const text = subDraft.value;
  subDraft.value = "";
  addingSub.value = false;
  if (text.trim()) await addSub(props.task, text);
}

async function clearConflict() {
  await api.resolveConflict(props.task.id);
  await rename(props.task, props.task.summary); // 触发一次刷新
}

function openInLark() {
  if (props.task.lark_url) openUrl(props.task.lark_url);
}

const subCount = () => subtasks[props.task.id]?.length ?? 0;

const canDrag = computed(() => !props.sub && !editing.value && !addingSub.value);

function startTaskDrag(event: DragEvent) {
  if (!canDrag.value) {
    event.preventDefault();
    return;
  }
  const target = event.target as HTMLElement | null;
  if (target?.closest("button, input, a")) {
    event.preventDefault();
    return;
  }
  if (!event.dataTransfer) return;
  event.dataTransfer.effectAllowed = "move";
  // WebView2 对自定义 MIME 类型支持不稳定，使用标准 text/plain。
  event.dataTransfer.setData("text/plain", props.task.id);
  event.dataTransfer.setData("text", props.task.id);
  dragUi.draggingId = props.task.id;
  (event.currentTarget as HTMLElement).classList.add("is-dragging");
}

function endTaskDrag(event: DragEvent) {
  (event.currentTarget as HTMLElement).classList.remove("is-dragging");
  noteTaskDrag();
}
</script>

<template>
  <div
    class="row"
    :class="{
      'is-done': task.done,
      sub,
      'drop-before': drop === 'before',
      'drop-after': drop === 'after',
    }"
    :data-id="task.id"
    :draggable="canDrag"
    @dragstart="startTaskDrag"
    @dragend="endTaskDrag"
  >
    <span
      v-if="!sub"
      class="drag-handle"
      aria-hidden="true"
      title="拖动排序，或拖到上方日期改期"
    >⋮⋮</span>
    <button
      class="check"
      :class="{ on: task.done }"
      :aria-label="task.done ? '标记为未完成' : '标记为完成'"
      :aria-pressed="task.done"
      @mousedown.stop
      @click="toggleDone(task)"
    >
      ✓
    </button>

    <div class="body">
      <input
        v-if="editing"
        ref="titleEl"
        v-model="draft"
        class="titleinput"
        aria-label="待办标题"
        draggable="false"
        @keydown.enter.prevent="commitEdit"
        @keydown.esc.prevent="editing = false"
        @blur="commitEdit"
      />
      <div
        v-else
        class="title"
        role="button"
        tabindex="0"
        @click="startEdit"
        @keydown.enter.prevent="startEdit"
      >
        {{ task.summary }}
      </div>

      <div v-if="task.remind_at || task.conflict || task.lark_guid || subCount()" class="meta">
        <span v-if="task.remind_at" class="tag">⏰ {{ timeStr(task.remind_at) }}</span>
        <button
          v-if="task.conflict"
          class="tag conflict"
          title="飞书侧也改了这条，已保留本地版本。点击确认。"
          @click="clearConflict"
        >
          冲突
        </button>
        <button
          v-if="task.lark_url"
          class="tag synced"
          title="在飞书中打开"
          @click="openInLark"
        >
          飞书
        </button>
        <span v-else-if="task.lark_guid" class="tag synced">已同步</span>
        <button v-if="subCount()" class="tag" @click="toggleExpand(task)">
          子待办 {{ subCount() }}
        </button>
      </div>

      <div v-if="addingSub" class="meta">
        <input
          v-model="subDraft"
          class="titleinput"
          placeholder="子待办，回车添加"
          aria-label="新子待办"
          autofocus
          @keydown.enter.prevent="commitSub"
          @keydown.esc.prevent="((addingSub = false), (subDraft = ''))"
          @blur="commitSub"
        />
      </div>
    </div>

    <div class="actions" @mousedown.stop>
      <button
        v-if="!sub"
        class="iconbtn"
        title="添加子待办"
        aria-label="添加子待办"
        @click="addingSub = true"
      >
        ＋
      </button>
      <button class="iconbtn" title="删除" aria-label="删除待办" @click="remove(task)">
        ✕
      </button>
    </div>
  </div>

  <template v-if="!sub && expanded[task.id]">
    <TaskRow
      v-for="s in subtasks[task.id] ?? []"
      :key="s.id"
      :task="s"
      sub
    />
  </template>
</template>
