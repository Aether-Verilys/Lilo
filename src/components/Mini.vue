<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../api";
import type { Side, Task } from "../types";
import { todayStr } from "../date";

/** 浮窗只关心今天的第一条未完成待办 */
const list = ref<Task[]>([]);
const cur = computed(() => list.value.find((t) => !t.done));
const left = computed(() => list.value.filter((t) => !t.done).length);

/** 吸附到哪一边；null = 自由浮动 */
const side = ref<Side | null>(null);
const collapsed = ref(false);
/** 折叠/展开动画期间加类，避免 CSS 过渡与窗口移动打架 */
const sliding = ref(false);

const win = getCurrentWindow();
let unlisten: UnlistenFn[] = [];
/** 移开后延迟折叠，避免鼠标扫过就收起来 */
let collapseTimer: number | undefined;
/**
 * `onMoved` 在 Windows 上会在拖动过程中连续触发，并不是「松手」事件。
 * 等位置稳定一小段时间再判定，避免刚碰到边缘就被程序抢走窗口位置。
 */
let snapTimer: number | undefined;
let moveGeneration = 0;

async function load() {
  list.value = await api.listTasks(todayStr());
}

async function complete() {
  const t = cur.value;
  if (!t) return;
  await api.updateTask(t.id, { done: true });
  await load();
}

async function collapse() {
  if (!side.value || collapsed.value || sliding.value) return;
  if (await api.snapPointerOver()) {
    api.snapDebug("collapse 跳过：光标还在窗口上");
    collapseTimer = window.setTimeout(collapse, 600);
    return;
  }
  sliding.value = true;
  try {
    await api.snapCollapse(side.value);
    collapsed.value = true;
    api.snapDebug(`collapse 完成 side=${side.value}`);
  } catch (err) {
    api.snapDebug(`collapse 失败: ${err}`);
  }
  sliding.value = false;
  if (collapsed.value && await api.snapPointerOver()) void expand();
}

async function expand() {
  if (!side.value || !collapsed.value || sliding.value) return;
  sliding.value = true;
  collapsed.value = false;
  try {
    await api.snapExpand(side.value);
    void api.pullOnView();
  } catch (err) {
    collapsed.value = true;
    api.snapDebug(`expand 失败: ${err}`);
  } finally {
    sliding.value = false;
  }
}

function onEnter() {
  clearTimeout(collapseTimer);
  if (collapsed.value) expand();
}

function onLeave() {
  if (!side.value || collapsed.value) {
    api.snapDebug(`onLeave 跳过 side=${side.value} collapsed=${collapsed.value}`);
    return;
  }
  clearTimeout(collapseTimer);
  api.snapDebug("onLeave 排定 600ms 后折叠");
  collapseTimer = window.setTimeout(collapse, 600);
}

/** 手动脱离吸附，回到自由浮动 */
async function unsnap() {
  clearTimeout(collapseTimer);
  if (collapsed.value && side.value) await api.snapExpand(side.value);
  await api.snapUnsnap();
  side.value = null;
  collapsed.value = false;
}

/** 通过按钮直接吸附，作为拖边自动吸附之外的稳定入口。 */
async function snapTo(next: Side) {
  clearTimeout(collapseTimer);
  sliding.value = true;
  try {
    await api.snapTo(next);
    side.value = next;
    collapsed.value = false;
  } finally {
    setTimeout(() => (sliding.value = false), 180);
  }
}

async function startDrag(event: PointerEvent) {
  if ((event.target as HTMLElement).closest("button")) return;
  clearTimeout(collapseTimer);
  await win.startDragging();
}

onMounted(async () => {
  await load();
  void api.pullOnView();
  unlisten.push(await listen("tasks-changed", load));

  api.snapDebug("mini mounted, 注册 onMoved");

  // Windows 拖动时会连续发 moved；位置稳定后判断是否落在吸附区。
  unlisten.push(
    await win.onMoved(async (e) => {
      // 折叠/展开是我们自己挪的窗口，不要把它当用户拖动
      if (sliding.value) {
        api.snapDebug(`onMoved 忽略(sliding) pos=${e.payload.x},${e.payload.y}`);
        return;
      }
      clearTimeout(snapTimer);
      const generation = ++moveGeneration;
      snapTimer = window.setTimeout(async () => {
        // 后续又发生过移动，说明用户仍在拖动，丢弃这次判定。
        if (generation !== moveGeneration || sliding.value) return;
        try {
          const s = await api.snapCheck();
          api.snapDebug(`拖动停止 pos=${e.payload.x},${e.payload.y} -> side=${s}`);
          side.value = s;
          collapsed.value = false;
          // 此时鼠标通常仍停在拖动条上；等待真正的 mouseleave 再折叠。
        } catch (err) {
          api.snapDebug(`onMoved snapCheck 失败: ${err}`);
        }
      }, 180);
    })
  );

  // 恢复上次的吸附状态
  const saved = await api.snapRestore();
  if (saved) {
    side.value = saved;
    await api.snapExpand(saved);
    onLeave();
  }
});

onUnmounted(() => {
  clearTimeout(collapseTimer);
  clearTimeout(snapTimer);
  unlisten.forEach((f) => f());
});

</script>

<template>
  <div
    class="mini"
    :class="[
      side ? `snap-${side}` : '',
      { collapsed, sliding },
    ]"
    @mouseenter="onEnter"
    @mouseleave="onLeave"
  >
    <!-- 折叠态露在屏幕内的那条窄边：唯一的命中区 -->
    <div v-if="collapsed" class="peek" :title="`还有 ${left} 条待办`">
      <span v-if="left" class="peek-n">{{ left }}</span>
    </div>

    <template v-else>
      <div class="bar" data-tauri-drag-region @pointerdown.left="startDrag">
        <span class="grow" data-tauri-drag-region>今天 · 剩 {{ left }}</span>
        <button
          v-if="!side"
          class="iconbtn"
          title="吸附到屏幕左侧"
          aria-label="吸附到屏幕左侧"
          @click="snapTo('left')"
        >
          ⇤
        </button>
        <button
          v-if="!side"
          class="iconbtn"
          title="吸附到屏幕右侧"
          aria-label="吸附到屏幕右侧"
          @click="snapTo('right')"
        >
          ⇥
        </button>
        <button
          v-if="side"
          class="iconbtn"
          title="取消吸附"
          aria-label="取消吸附"
          @click="unsnap"
        >
          ⇱
        </button>
        <button class="iconbtn" aria-label="隐藏浮窗" @click="win.hide()">✕</button>
      </div>

      <div class="cur">
        <template v-if="cur">
          <button class="check" aria-label="标记为完成" @click="complete">✓</button>
          <span class="t">{{ cur.summary }}</span>
        </template>
        <span v-else class="t" style="color: var(--fg-faint)">今天都做完了</span>
      </div>
    </template>
  </div>
</template>
