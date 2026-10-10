<script setup lang="ts">
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();

function closeDialog() {
  if (!store.updateDownloading) store.updateDialogVisible = false;
}
</script>

<template>
  <el-dialog
    :model-value="store.updateDialogVisible"
    title="发现新版本"
    width="min(400px, 92vw)"
    :close-on-click-modal="!store.updateDownloading"
    :close-on-press-escape="!store.updateDownloading"
    :show-close="!store.updateDownloading"
    @update:model-value="closeDialog"
  >
    <div v-if="store.updateAvailable" class="sf-update-dialog">
      <div class="sf-update-version">v{{ store.updateAvailable.version }}</div>
      <div class="sf-update-note">更新后软件会自动重启。</div>
      <template v-if="store.updateDownloading">
        <el-progress :percentage="store.updateProgress" :stroke-width="8" />
        <div class="sf-update-status">{{ store.updateStatus }}</div>
      </template>
    </div>
    <template #footer>
      <el-button v-if="!store.updateDownloading" @click="closeDialog">关闭</el-button>
      <el-button type="primary" :loading="store.updateDownloading" @click="store.installUpdate()">
        {{ store.updateDownloading ? "更新中" : "立即更新" }}
      </el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.sf-update-dialog {
  display: grid;
  gap: var(--space-sm);
}

.sf-update-version {
  color: var(--color-ink);
  font-family: var(--font-mono);
  font-size: var(--text-lg);
  font-weight: 700;
}

.sf-update-note,
.sf-update-status {
  color: var(--sf-text-secondary);
  font-size: var(--text-xs);
}

.sf-update-status {
  text-align: center;
}
</style>
