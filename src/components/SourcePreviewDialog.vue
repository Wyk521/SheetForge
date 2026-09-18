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
    <div style="font-size: 12px; font-weight: 600; margin-bottom: 10px">
      {{ store.sourcePreviewTitle }}
    </div>
    <div v-if="store.sourcePreviewLoading" style="padding: 50px 0; text-align: center; color: var(--sf-text-muted)">
      正在读取源数据…
    </div>
    <template v-else-if="store.sourcePreview">
      <PreviewTableGrid :preview="store.sourcePreview" :max-height="520" />
    </template>
    <div v-else style="padding: 50px 0; text-align: center; color: var(--sf-text-muted)">
      暂无可预览内容
    </div>
    <template #footer>
      <el-button @click="store.closeSourcePreview()">关闭</el-button>
    </template>
  </el-dialog>
</template>
