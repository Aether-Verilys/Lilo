<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import DayView from "./components/DayView.vue";
import Overview from "./components/Overview.vue";
import Settings from "./components/Settings.vue";
import SyncBar from "./components/SyncBar.vue";
import Mini from "./components/Mini.vue";
import { checkScopes, holdOpen, initListeners, pullOnView, refreshSync, reload, settingsOpen, view } from "./store";
import { api } from "./api";

/** 窗口 label 由 Tauri 保证，比开发服务器可能改写的 URL 查询参数可靠。 */
const win = getCurrentWindow();
const isMini = win.label === "mini";
let unlistenMoved: UnlistenFn | undefined;
let unlistenResized: UnlistenFn | undefined;
let snapTimer: number | undefined;
let collapseTimer: number | undefined;
let resizeTimer: number | undefined;
let programmaticUntil = 0;
const snapSide = ref<"left" | "right" | null>(null);
const collapsed = ref(false);
const sliding = ref(false);

async function collapseMain() {
  if (!snapSide.value || collapsed.value || sliding.value || holdOpen.value) return;
  if (await api.snapPointerOver()) {
    scheduleCollapse();
    return;
  }
  sliding.value = true;
  try {
    programmaticUntil = Date.now() + 500;
    await api.snapCollapse(snapSide.value);
    collapsed.value = true;
  } finally {
    sliding.value = false;
    if (collapsed.value && await api.snapPointerOver()) void expandMain();
  }
}

async function expandMain() {
  clearTimeout(collapseTimer);
  if (!snapSide.value || !collapsed.value || sliding.value) return;
  sliding.value = true;
  try {
    programmaticUntil = Date.now() + 500;
    await api.snapExpand(snapSide.value);
    collapsed.value = false;
    pullOnView();
  } finally {
    sliding.value = false;
  }
}

function scheduleCollapse() {
  if (!snapSide.value || collapsed.value || holdOpen.value) {
    clearTimeout(collapseTimer);
    return;
  }
  clearTimeout(collapseTimer);
  collapseTimer = window.setTimeout(() => {
    void collapseMain();
  }, 700);
}

function onMainEnter() {
  clearTimeout(collapseTimer);
  if (collapsed.value) void expandMain();
}

function onMainLeave() {
  scheduleCollapse();
}

watch(holdOpen, (on) => {
  if (on) {
    clearTimeout(collapseTimer);
    return;
  }
  scheduleCollapse();
});

onMounted(async () => {
  if (isMini) return;
  await initListeners();
  await reload();
  await refreshSync();
  await checkScopes();
  pullOnView();

  // 主窗口使用系统标题栏拖动。停止移动后，靠近左右屏幕边缘就自动贴平。
  unlistenMoved = await win.onMoved(() => {
    if (sliding.value || Date.now() < programmaticUntil) return;
    clearTimeout(snapTimer);
    snapTimer = window.setTimeout(async () => {
      if (sliding.value || Date.now() < programmaticUntil) return;
      const side = await api.snapCheck();
      programmaticUntil = Date.now() + 500;
      snapSide.value = side;
      collapsed.value = false;
      if (side) scheduleCollapse();
    }, 180);
  });

  // 贴边后拖宽度，窗口必须重新贴平，否则会按旧宽度滑出屏幕。
  unlistenResized = await win.onResized(() => {
    if (!snapSide.value || sliding.value || Date.now() < programmaticUntil) return;
    clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(async () => {
      if (!snapSide.value || sliding.value) return;
      programmaticUntil = Date.now() + 400;
      if (collapsed.value) await api.snapCollapse(snapSide.value);
      else await api.snapExpand(snapSide.value);
    }, 80);
  });

  // 重新启动时恢复主窗口上次吸附的屏幕侧边。
  const saved = await api.snapRestore();
  if (saved) {
    snapSide.value = saved;
    programmaticUntil = Date.now() + 800;
    await api.snapTo(saved);
    scheduleCollapse();
  }
});

onUnmounted(() => {
  clearTimeout(snapTimer);
  clearTimeout(collapseTimer);
  clearTimeout(resizeTimer);
  unlistenMoved?.();
  unlistenResized?.();
});
</script>

<template>
  <Mini v-if="isMini" />
  <div
    v-else
    class="app"
    :class="[
      snapSide ? `snap-${snapSide}` : '',
      { 'snap-collapsed': collapsed },
    ]"
    @mouseenter="onMainEnter"
    @mouseleave="onMainLeave"
  >
    <div v-if="collapsed" class="main-edge-peek" aria-label="展开 Lilo" />
    <Settings v-if="settingsOpen" @close="settingsOpen = false" />
    <template v-else>
      <DayView v-if="view.mode === 'day'" />
      <Overview v-else />
      <SyncBar />
    </template>
  </div>
</template>
