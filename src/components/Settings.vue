<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api } from "../api";
import type { LarkAccount } from "../types";

defineEmits<{ close: [] }>();

const autostart = ref(false);
const opacity = ref(90);
const opacityPresets = [100, 95, 90, 85, 80] as const;
const account = ref<LarkAccount | null>(null);
const accountError = ref("");

onMounted(async () => {
  autostart.value = await api.getAutostart();
  opacity.value = await api.getPanelOpacity();
  try {
    account.value = await api.larkAccount();
  } catch (e) {
    accountError.value = e instanceof Error ? e.message : String(e);
  }
});

async function toggleAutostart() {
  const next = !autostart.value;
  await api.setAutostart(next);
  autostart.value = next;
}

async function pickOpacity(percent: number) {
  opacity.value = await api.setPanelOpacity(percent);
}

function accountText() {
  if (accountError.value) return "未能读取飞书账号";
  if (!account.value) return "正在读取…";
  if (account.value.ready && account.value.name) return account.value.name;
  if (account.value.name) return `${account.value.name}（未授权）`;
  return "未登录飞书";
}
</script>

<template>
  <div class="settings">
    <div class="head">
      <h1>设置</h1>
      <button class="iconbtn" title="关闭" aria-label="关闭" @click="$emit('close')">✕</button>
    </div>
    <div class="settings-body">
      <label class="settings-row" @click.prevent="toggleAutostart">
        <span class="settings-label">开机自启</span>
        <span class="toggle" :class="{ on: autostart }" role="switch" :aria-checked="autostart" />
      </label>
      <div class="settings-block">
        <span class="settings-label">不透明度</span>
        <div class="opacity-picks" role="radiogroup" aria-label="不透明度">
          <button
            v-for="p in opacityPresets"
            :key="p"
            class="opacity-pick"
            :class="{ on: opacity === p }"
            role="radio"
            :aria-checked="opacity === p"
            @click="pickOpacity(p)"
          >
            {{ p }}%
          </button>
        </div>
      </div>
      <div class="settings-row settings-account">
        <span class="settings-label">飞书账号</span>
        <span class="settings-value" :class="{ muted: !account?.ready }">{{ accountText() }}</span>
      </div>
    </div>
  </div>
</template>
