<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import TaskRow from "./TaskRow.vue";
import { isPast, label, todayStr } from "../date";
import {
  add,
  dragUi,
  holdOpen,
  noteTaskDrag,
  reorder,
  scrollDays,
  selectDay,
  tasks,
  view,
} from "../store";
import WeekBoard from "./WeekBoard.vue";

const composing = ref(false);
const overId = ref<string | null>(null);
const overAfter = ref(false);
const quick = ref(false);
const quickDraft = ref("");
const quickEl = ref<HTMLInputElement | null>(null);

const open = computed(() => tasks.value.filter((t) => !t.done));
const done = computed(() => tasks.value.filter((t) => t.done));

async function toInbox() {
  await selectDay(view.day === null ? todayStr() : null);
}

function onSelect(day: string) {
  void selectDay(day);
}

/** 空白处双击：当前天（收集箱则切到今天）在列表里起一条新待办 */
async function startQuickAdd(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (target?.closest(".row, input, button, a")) return;
  if (quick.value) {
    quickEl.value?.focus();
    return;
  }
  if (view.day === null) await selectDay(todayStr());
  quick.value = true;
  holdOpen.value = true;
  quickDraft.value = "";
  await nextTick();
  quickEl.value?.focus();
}

let committing = false;
async function commitQuick() {
  if (composing.value || committing) return;
  committing = true;
  const text = quickDraft.value.trim();
  quick.value = false;
  holdOpen.value = false;
  quickDraft.value = "";
  try {
    if (text) await add(text);
  } finally {
    committing = false;
  }
}

function cancelQuick() {
  quick.value = false;
  holdOpen.value = false;
  quickDraft.value = "";
}

function groupOf(id: string) {
  if (open.value.some((t) => t.id === id)) return open.value;
  if (done.value.some((t) => t.id === id)) return done.value;
  return null;
}

function rowFromEvent(event: DragEvent) {
  return (event.target as HTMLElement | null)?.closest(".row:not(.sub)") as HTMLElement | null;
}

function onListDragOver(event: DragEvent) {
  if (!dragUi.draggingId) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  const row = rowFromEvent(event);
  const over = row?.dataset.id ?? null;
  if (!over || over === dragUi.draggingId || !groupOf(dragUi.draggingId)?.some((t) => t.id === over)) {
    overId.value = null;
    return;
  }
  const rect = row!.getBoundingClientRect();
  overId.value = over;
  overAfter.value = event.clientY > rect.top + rect.height / 2;
}

function onListDragLeave(event: DragEvent) {
  const next = event.relatedTarget as Node | null;
  if (next && (event.currentTarget as HTMLElement).contains(next)) return;
  overId.value = null;
}

async function onListDrop(event: DragEvent) {
  const id = event.dataTransfer?.getData("text/plain")
    || event.dataTransfer?.getData("text")
    || dragUi.draggingId;
  const targetId = overId.value;
  const after = overAfter.value;
  overId.value = null;
  if (!id) return;
  const group = groupOf(id);
  if (!group) return;
  event.preventDefault();
  noteTaskDrag();
  const ids = group.map((t) => t.id);
  const from = ids.indexOf(id);
  if (from < 0) return;
  ids.splice(from, 1);
  let to = ids.length;
  if (targetId && targetId !== id) {
    const at = ids.indexOf(targetId);
    if (at >= 0) to = after ? at + 1 : at;
  }
  ids.splice(to, 0, id);
  if (ids.every((x, i) => x === group[i].id)) return;
  await reorder(ids);
}

function dropSide(id: string) {
  if (overId.value !== id) return null;
  return overAfter.value ? "after" : "before";
}
</script>

<template>
  <div class="head">
    <button class="iconbtn" aria-label="日期条往前滚一天" @click="scrollDays(-1)">‹</button>
    <h1 :class="{ past: isPast(view.day) }">{{ label(view.day) }}</h1>
    <button class="iconbtn" aria-label="日期条往后滚一天" @click="scrollDays(1)">›</button>
    <button
      class="iconbtn"
      :class="{ on: view.day === null }"
      title="收集箱"
      aria-label="收集箱"
      @click="toInbox"
    >
      ☰
    </button>
    <button
      class="iconbtn"
      title="总览"
      aria-label="总览"
      @click="view.mode = 'overview'"
    >
      ▦
    </button>
  </div>

  <WeekBoard :selected="view.day" :anchor="view.anchor" @select="onSelect" />

  <div
    class="list"
    @dblclick="startQuickAdd"
    @dragover="onListDragOver"
    @dragleave="onListDragLeave"
    @drop="onListDrop"
  >
    <p v-if="!tasks.length && !quick" class="empty">
      {{ view.day === null ? "收集箱是空的" : "这天还没有待办" }}<br />
      双击空白处添加
    </p>

    <TaskRow v-for="t in open" :key="t.id" :task="t" :drop="dropSide(t.id)" />

    <div v-if="quick" class="row composing">
      <span class="drag-handle" aria-hidden="true">⋮⋮</span>
      <span class="check" aria-hidden="true" />
      <div class="body">
        <input
          ref="quickEl"
          v-model="quickDraft"
          class="titleinput"
          placeholder="新待办，回车保存"
          aria-label="新待办"
          @compositionstart="composing = true"
          @compositionend="composing = false"
          @keydown.enter.prevent="commitQuick"
          @keydown.esc.prevent="cancelQuick"
          @blur="commitQuick"
        />
      </div>
    </div>

    <template v-if="done.length">
      <div class="meta" style="padding: 8px 6px 2px">已完成 {{ done.length }}</div>
      <TaskRow v-for="t in done" :key="t.id" :task="t" :drop="dropSide(t.id)" />
    </template>
  </div>
</template>
