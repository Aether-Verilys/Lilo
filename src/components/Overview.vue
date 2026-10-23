<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { api } from "../api";
import type { DayCount } from "../types";
import { fmt, monthBounds, parse, todayStr } from "../date";
import { selectDay, view } from "../store";

/** 当前展示的月份锚点日期 */
const anchor = ref(view.day ?? todayStr());
const counts = ref<Record<string, [number, number]>>({});

const WD = ["日", "一", "二", "三", "四", "五", "六"];

async function load() {
  const [from, to] = monthBounds(anchor.value);
  const rows: DayCount[] = await api.dayCounts(from, to);
  const m: Record<string, [number, number]> = {};
  for (const [day, open, total] of rows) m[day] = [open, total];
  counts.value = m;
}

onMounted(load);
watch(anchor, load);

const title = computed(() => {
  const d = parse(anchor.value);
  return `${d.getFullYear()}年${d.getMonth() + 1}月`;
});

/** 月历网格：前面补空格对齐星期 */
const cells = computed(() => {
  const d = parse(anchor.value);
  const first = new Date(d.getFullYear(), d.getMonth(), 1);
  const days = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
  const out: (string | null)[] = Array(first.getDay()).fill(null);
  for (let i = 1; i <= days; i++) {
    out.push(fmt(new Date(d.getFullYear(), d.getMonth(), i)));
  }
  return out;
});

function shiftMonth(n: number) {
  const d = parse(anchor.value);
  anchor.value = fmt(new Date(d.getFullYear(), d.getMonth() + n, 1));
}

async function pick(day: string) {
  view.mode = "day";
  await selectDay(day);
}
</script>

<template>
  <div class="head">
    <button class="iconbtn" aria-label="上个月" @click="shiftMonth(-1)">‹</button>
    <h1>{{ title }}</h1>
    <button class="iconbtn" aria-label="下个月" @click="shiftMonth(1)">›</button>
    <button
      class="iconbtn"
      title="回到按天视图"
      aria-label="回到按天视图"
      @click="view.mode = 'day'"
    >
      ✕
    </button>
  </div>

  <div class="overview">
    <div class="month">
      <div v-for="w in WD" :key="w" class="wd">{{ w }}</div>
      <template v-for="(c, i) in cells" :key="i">
        <div v-if="!c" class="cell blank" />
        <button
          v-else
          class="cell"
          :class="{ today: c === todayStr() }"
          :aria-label="`${c}，未完成 ${counts[c]?.[0] ?? 0} 条`"
          @click="pick(c)"
        >
          <span>{{ parse(c).getDate() }}</span>
          <span class="n" :class="{ has: (counts[c]?.[0] ?? 0) > 0 }">
            {{ counts[c] ? counts[c][0] : "" }}
          </span>
        </button>
      </template>
    </div>
  </div>
</template>
