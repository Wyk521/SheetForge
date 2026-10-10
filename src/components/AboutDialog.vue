<script setup lang="ts">
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();

function openUpdate() {
  if (store.updateAvailable) {
    store.showAbout = false;
    store.updateDialogVisible = true;
    return;
  }
  void store.checkUpdate(false);
}
</script>

<template>
  <el-dialog v-model="store.showAbout" title="关于" width="min(380px, 92vw)" :close-on-click-modal="true">
    <div class="sf-about-layout">
      <div class="sf-logo sf-about-logo">表</div>
      <div class="sf-about-copy">
        <div class="sf-about-title">表格合并</div>
        <div class="sf-about-meta">
          <span>格式</span><span>Excel / CSV / ODS</span>
          <span>处理</span><span>本机</span>
        </div>
      </div>
    </div>
    <template #footer>
      <el-button size="small" :loading="store.updateChecking" @click="openUpdate">
        {{ store.updateText }}
      </el-button>
      <el-button size="small" @click="store.openLog()">查看日志</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.sf-about-layout {
  display: flex;
  align-items: center;
  gap: var(--space-md);
}

.sf-about-logo {
  width: 3rem;
  height: 3rem;
  font-size: var(--text-lg);
}

.sf-about-copy {
  min-width: 0;
}

.sf-about-title {
  color: var(--color-ink);
  font-family: var(--font-display);
  font-size: var(--text-lg);
  font-weight: 700;
}

.sf-about-meta {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--space-3xs) var(--space-sm);
  margin-top: var(--space-xs);
  color: var(--sf-text-secondary);
  font-size: var(--text-2xs);
}

.sf-about-meta span:nth-child(odd) {
  color: var(--sf-text-muted);
}
</style>
