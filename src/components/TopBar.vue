<script setup lang="ts">
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();

const stages = [
  { index: 0, code: "01", label: "数据源" },
  { index: 1, code: "02", label: "合并工作区" },
  { index: 2, code: "03", label: "输出预览" },
];
</script>

<template>
  <header class="sf-topbar">
    <nav class="sf-route-nav" aria-label="合并流程">
      <button
        v-for="stage in stages"
        :key="stage.index"
        type="button"
        class="sf-route-stop"
        :class="{
          'is-active': store.activePage === stage.index,
          'is-complete': store.activePage > stage.index,
        }"
        :aria-current="store.activePage === stage.index ? 'step' : undefined"
        @click="store.activePage = stage.index"
      >
        <span class="sf-route-code">{{ stage.code }}</span>
        <span class="sf-route-node" aria-hidden="true"></span>
        <span class="sf-route-copy">
          <strong>{{ stage.label }}</strong>
        </span>
      </button>
    </nav>

    <div class="sf-topbar-actions" aria-label="方案与设置">
      <el-button text :disabled="!store.hasSources || store.busy" @click="store.saveScheme()">
        保存方案
      </el-button>
      <el-button text :disabled="store.busy" @click="store.openScheme()">打开方案</el-button>
      <el-button text :disabled="store.busy" @click="store.openDatabaseConnections()">数据库连接</el-button>
      <el-button text @click="store.showAbout = true">关于</el-button>
    </div>
  </header>
</template>

<style scoped>
.sf-topbar-actions {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}
</style>
