<script setup lang="ts">
import { computed, ref } from "vue";
import { api } from "../api";
import { checkScopes, grantWriteScopes, missingScopes, settingsOpen, sync } from "../store";
import { timeStr } from "../date";

const text = computed(() => {
  const s = sync.value;
  if (s.message && s.state !== "idle") return s.message;
  switch (s.state) {
    case "syncing":
      return "正在同步飞书…";
    case "offline":
      return "离线，改动已存本地";
    case "unauthorized":
      return "飞书未授权";
    case "disabled":
      return "飞书同步已关闭";
    case "error":
      return "同步出错";
    default: {
      if (!s.last_sync_at) return "等待同步";
      return `已同步 · ${timeStr(s.last_sync_at)}`;
    }
  }
});

const granting = ref(false);
const grantError = ref("");

function errText(e: unknown): string {
  if (typeof e === "string" && e.trim()) return e;
  if (e instanceof Error && e.message) return e.message;
  if (e && typeof e === "object" && "message" in e) {
    const m = (e as { message: unknown }).message;
    if (typeof m === "string" && m.trim()) return m;
  }
  return "授权失败";
}

async function grantWrite() {
  if (granting.value) return;
  granting.value = true;
  grantError.value = "";
  try {
    const still = await grantWriteScopes();
    if (still.length) grantError.value = "授权未完成，仍缺少写入权限";
  } catch (e) {
    grantError.value = errText(e);
  } finally {
    granting.value = false;
  }
}

async function syncNow() {
  try {
    await api.syncNow();
  } finally {
    await checkScopes();
  }
}
</script>

<template>
  <div v-if="missingScopes.length" class="notice">
    <div class="notice-row">
      <span>飞书任务缺少写入权限，当前只能读取。</span>
      <button class="notice-btn" :disabled="granting" @click="grantWrite">
        {{ granting ? "等待授权…" : "一键授权" }}
      </button>
    </div>
    <p v-if="granting" class="notice-hint">已打开浏览器，请在飞书页面确认后回到这里。</p>
    <p v-if="grantError" class="notice-err">{{ grantError }}</p>
  </div>

  <div class="syncbar">
    <button
      class="iconbtn sync-gear"
      title="设置"
      aria-label="设置"
      @click="settingsOpen = true"
    >
      ⚙
    </button>
    <span class="dot" :class="sync.state" />
    <span class="msg">{{ text }}</span>
    <span v-if="sync.pending > 0" :title="`${sync.pending} 条待推送`">
      ↑{{ sync.pending }}
    </span>
    <button
      v-if="sync.state !== 'disabled'"
      title="立即同步"
      @click="syncNow()"
    >
      同步
    </button>
  </div>
</template>
