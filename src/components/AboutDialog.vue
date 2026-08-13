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
  <el-dialog v-model="store.showAbout" title="关于表格合并" width="440px" :close-on-click-modal="true">
    <div style="display: flex; gap: 14px; align-items: flex-start">
      <div class="sf-logo" style="width: 48px; height: 48px; font-size: 24px">表</div>
      <div>
        <div style="font-size: 17px; font-weight: 700">表格合并</div>
        <div style="color: var(--sf-text-muted); font-size: 11px; margin: 4px 0 10px">
          本地批量表格工具 · 使用 Rust + Tauri 构建，表格内容不会上传
        </div>
        <div style="color: var(--sf-text-secondary); font-size: 12px; line-height: 1.8">
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
