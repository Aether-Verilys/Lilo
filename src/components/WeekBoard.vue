<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { addDays, label, parse, todayStr } from "../date";
import { api } from "../api";
import { dragUi, noteTaskDrag, tasks } from "../store";

const props = defineProps<{ selected: string | null; anchor: string }>();
const emit = defineEmits<{ select: [day: string] }>();
const overDay = ref<string | null>(null);
/** 滚动的 7 天：从锚点那天起往后数六天。滚动条移动不改选中日。 */
const days = computed(() =>
  Array.from({ length: 7 }, (_, i) => addDays(props.anchor, i))
);

/** 每天的未完成数，用来点蓝点 */
const openCounts = ref<Record<string, number>>({});

async function loadCounts() {
  const list = days.value;
  const rows = await api.dayCounts(list[0], list[list.length - 1]);
  const m: Record<string, number> = {};
  for (const [day, open] of rows) m[day] = open;
  openCounts.value = m;
}

// 换天要重算窗口，任务增删改完 store 会换 tasks 引用，跟着刷新计数
watch(days, loadCounts, { immediate: true });
watch(tasks, loadCounts);

function hasOpen(day: string) { return (openCounts.value[day] ?? 0) > 0; }

function dayFromEvent(event: DragEvent) {
  return (event.target as HTMLElement | null)?.closest(".week-day") as HTMLElement | null;
}

function onDragOver(event: DragEvent) {
  if (!dragUi.draggingId) return;
  const cell = dayFromEvent(event);
  if (!cell?.dataset.day) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  overDay.value = cell.dataset.day;
}

function onDragLeave(event: DragEvent) {
  const next = event.relatedTarget as Node | null;
  const grid = event.currentTarget as HTMLElement;
  if (next && grid.contains(next)) return;
  overDay.value = null;
}

async function onDrop(event: DragEvent) {
  const id = event.dataTransfer?.getData("text/plain")
    || event.dataTransfer?.getData("text")
    || dragUi.draggingId;
  const day = dayFromEvent(event)?.dataset.day ?? overDay.value;
  overDay.value = null;
  if (!id || !day) return;
  event.preventDefault();
  noteTaskDrag();
  await api.updateTask(id, { day });
  emit("select", day);
}

function dayName(day: string) { return label(day).split(" ").pop() ?? ""; }
function dayDate(day: string) { const d = parse(day); return `${d.getMonth() + 1}/${d.getDate()}`; }
function isToday(day: string) { return day === todayStr(); }
function isWeekend(day: string) { const w = parse(day).getDay(); return w === 0 || w === 6; }
</script>

<template>
  <section class="week-board" aria-label="未来 7 天待办">
    <div class="week-hint">7 天 · 拖任务到日期改期</div>
    <div
      class="week-grid"
      @dragover="onDragOver"
      @dragleave="onDragLeave"
      @drop="onDrop"
    >
      <div
        v-for="day in days"
        :key="day"
        class="week-day"
        :class="{
          today: isToday(day),
          selected: selected === day,
          over: overDay === day,
          weekend: isWeekend(day),
        }"
        :data-day="day"
        @click="emit('select', day)"
      >
        <div class="week-day-head">
          <strong>{{ dayName(day) }}</strong><span>{{ dayDate(day) }}</span>
        </div>
        <span
          v-if="hasOpen(day)"
          class="week-dot"
          :title="`${openCounts[day]} 条未完成`"
          :aria-label="`${openCounts[day]} 条未完成`"
        />
        <span v-else class="week-dot-empty" aria-hidden="true" />
      </div>
    </div>
  </section>
</template>
