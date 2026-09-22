<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();

async function openUpdatePage() {
  if (store.updateUrl) {
    await openUrl(store.updateUrl);
    store.updateUrl = null;
    store.updateText = "检查更新";
  } else {
    await store.checkUpdate();
  }
}
</script>

<template>
  <el-dialog v-model="store.showAbout" title="关于表格合并" width="min(440px, 92vw)" :close-on-click-modal="true">
    <div class="sf-about-layout">
      <div class="sf-logo sf-about-logo">表</div>
      <div class="sf-about-copy">
        <div class="sf-about-title">表格合并</div>
        <div class="sf-about-subtitle">
          本地批量表格工具 · 使用 Rust + Tauri 构建，表格内容不会上传
        </div>
        <div class="sf-about-features">
          <div>· 递归扫描文件夹，支持 xlsx / xlsm / xls / xlsb / ods / csv / tsv</div>
          <div>· 五种合并方式：并集、交集、修正表头、按键汇总、横向关联</div>
          <div>· 所有处理都在本机完成，仅“检查更新”会联网</div>
        </div>
      </div>
    </div>
    <template #footer>
      <el-button @click="openUpdatePage">{{ store.updateText }}</el-button>
      <el-button @click="store.openLog()">查看日志</el-button>
      <el-button type="primary" @click="store.showAbout = false">关闭</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.sf-about-layout {
  display: flex;
  align-items: flex-start;
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

.sf-about-subtitle {
  margin: var(--space-2xs) 0 var(--space-sm);
  color: var(--sf-text-muted);
  font-size: var(--text-2xs);
  line-height: 1.55;
}

.sf-about-features {
  color: var(--sf-text-secondary);
  font-size: var(--text-xs);
  line-height: 1.8;
}
</style>
