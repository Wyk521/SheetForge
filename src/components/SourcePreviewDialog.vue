<script setup lang="ts">
import { useMergeStore } from "../stores/merge";
import PreviewTableGrid from "./PreviewTableGrid.vue";

const store = useMergeStore();
</script>

<template>
  <el-dialog
    :model-value="store.sourcePreviewVisible"
    title="源数据预览"
    width="min(1120px, 92vw)"
    top="5vh"
    destroy-on-close
    @close="store.closeSourcePreview()"
  >
    <div class="sf-preview-dialog-title">
      {{ store.sourcePreviewTitle }}
    </div>
    <div v-if="store.sourcePreviewLoading" class="sf-dialog-empty">
      正在读取源数据…
    </div>
    <template v-else-if="store.sourcePreview">
      <PreviewTableGrid :preview="store.sourcePreview" :max-height="520" />
    </template>
    <div v-else class="sf-dialog-empty">
      暂无可预览内容
    </div>
    <template #footer>
      <el-button @click="store.closeSourcePreview()">关闭</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.sf-preview-dialog-title {
  margin-bottom: var(--space-sm);
  font-size: var(--text-xs);
  font-weight: 700;
}

.sf-dialog-empty {
  padding: var(--space-2xl) 0;
  color: var(--sf-text-muted);
  text-align: center;
}
</style>
