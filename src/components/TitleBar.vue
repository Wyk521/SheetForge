<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";

const isTauri = Boolean((window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
const appWindow = isTauri ? getCurrentWindow() : null;
const isMax = ref(false);
let unlisten: (() => void) | undefined;

onMounted(async () => {
  try {
    isMax.value = (await appWindow?.isMaximized()) ?? false;
  } catch {
    /* 窗口 API 不可用时静默降级 */
  }
  try {
    unlisten = await appWindow?.onResized(() => {
      void appWindow?.isMaximized().then((v) => (isMax.value = v));
    });
  } catch {
    /* 忽略 */
  }
});

onUnmounted(() => {
  try {
    unlisten?.();
  } catch {
    /* 忽略 */
  }
});
</script>

<template>
  <div class="sf-titlebar">
    <div class="sf-titlebar-drag" data-tauri-drag-region>
      <span class="sf-titlebar-title" data-tauri-drag-region>表格合并</span>
    </div>
    <div class="sf-titlebar-controls">
      <button
        class="sf-win-btn"
        title="最小化"
        aria-label="最小化"
        @click="appWindow?.minimize()"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 4.5h10v1H0z" fill="currentColor" />
        </svg>
      </button>
      <button
        class="sf-win-btn"
        :title="isMax ? '还原' : '最大化'"
        :aria-label="isMax ? '还原' : '最大化'"
        @click="appWindow?.toggleMaximize()"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            v-if="!isMax"
            d="M0 0h10v10H0zm1 1v8h8V1z"
            fill="currentColor"
            fill-rule="evenodd"
          />
          <path v-else d="M3 0v3H0v7h7V7h3V0zM1.5 4.5h4v4h-4zm5-3h-3v1h2v2h1z" fill="currentColor" />
        </svg>
      </button>
      <button
        class="sf-win-btn sf-win-close"
        title="关闭"
        aria-label="关闭"
        @click="appWindow?.close()"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M1.4 0L5 3.6 8.6 0 10 1.4 6.4 5 10 8.6 8.6 10 5 6.4 1.4 10 0 8.6 3.6 5 0 1.4z" fill="currentColor" />
        </svg>
      </button>
    </div>
  </div>
</template>

<style scoped>
.sf-titlebar {
  flex-shrink: 0;
  height: 36px;
  display: flex;
  align-items: stretch;
  background: var(--sf-bg-card);
  border-bottom: 1px solid var(--sf-border);
  user-select: none;
}

.sf-titlebar-drag {
  flex: 1;
  display: flex;
  align-items: center;
  padding-left: 14px;
  cursor: default;
}

.sf-titlebar-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--sf-text-secondary);
  letter-spacing: 0.3px;
}

.sf-titlebar-controls {
  display: flex;
  align-items: stretch;
}

.sf-win-btn {
  width: 46px;
  height: 100%;
  border: none;
  background: transparent;
  color: var(--sf-text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  transition: background 0.12s, color 0.12s;
}

.sf-win-btn:hover {
  background: rgba(0, 0, 0, 0.06);
  color: var(--sf-text);
}

.sf-win-close:hover {
  background: #e81123;
  color: #fff;
}
</style>
