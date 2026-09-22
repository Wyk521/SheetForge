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
    <div
      class="sf-titlebar-drag"
      data-tauri-drag-region
      :aria-label="isMax ? '应用标题栏，窗口已最大化' : '应用标题栏'"
    >
      <span class="sf-titlebar-brand" data-tauri-drag-region>SheetForge</span>
      <span class="sf-titlebar-separator" data-tauri-drag-region aria-hidden="true">/</span>
      <span class="sf-titlebar-title" data-tauri-drag-region>桌面数据合并工作台</span>
      <span class="sf-titlebar-mark" data-tauri-drag-region>
        <span aria-hidden="true"></span>
        本地应用
      </span>
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
  background: var(--color-canvas);
  border-bottom: 1px solid var(--sf-border);
  user-select: none;
}

.sf-titlebar-drag {
  flex: 1;
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  padding-inline-start: var(--space-md);
  cursor: default;
}

.sf-titlebar-brand,
.sf-titlebar-title {
  font-size: var(--text-xs);
  font-weight: 700;
  color: var(--sf-text-secondary);
  letter-spacing: 0.04em;
}

.sf-titlebar-brand {
  font-family: var(--font-display);
  color: var(--color-ink);
  letter-spacing: -0.01em;
}

.sf-titlebar-separator {
  color: var(--color-rule-strong);
}

.sf-titlebar-mark {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2xs);
  margin-inline-start: var(--space-xs);
  color: var(--color-muted);
  font-family: var(--font-mono);
  font-size: var(--text-2xs);
  letter-spacing: 0.08em;
}

.sf-titlebar-mark > span {
  width: 6px;
  height: 6px;
  border-radius: var(--radius-round);
  background: var(--color-accent);
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
  color: var(--color-ink-2);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  transition: background-color var(--dur-micro) var(--ease-out), color var(--dur-micro) var(--ease-out), transform var(--dur-micro) var(--ease-out);
}

.sf-win-btn:active {
  transform: translateY(1px);
}

@media (hover: hover) and (pointer: fine) {
  .sf-win-btn:hover {
    background: var(--color-paper-2);
    color: var(--color-ink);
  }

  .sf-win-close:hover {
    background: var(--color-accent);
    color: var(--color-accent-ink);
  }
}

@media (max-width: 48rem) {
  .sf-titlebar-separator,
  .sf-titlebar-title,
  .sf-titlebar-mark {
    display: none;
  }
}
</style>
