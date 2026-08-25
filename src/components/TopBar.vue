<script setup lang="ts">
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();
</script>

<template>
  <div class="sf-topbar">
    <el-menu
      mode="horizontal"
      :default-active="String(store.activePage)"
      :ellipsis="false"
      class="sf-nav"
      @select="(index: string) => (store.activePage = Number(index))"
    >
      <el-menu-item index="0">01 数据源</el-menu-item>
      <el-menu-item index="1">02 合并规则</el-menu-item>
      <el-menu-item index="2">03 预览与检查</el-menu-item>
    </el-menu>
    <div style="flex: 1"></div>
    <el-button text :disabled="!store.hasSources || store.busy" @click="store.saveScheme()">
      保存方案
    </el-button>
    <el-button text :disabled="store.busy" @click="store.openScheme()">打开方案</el-button>
    <el-button text :disabled="store.busy" @click="store.openDatabaseConnections()">数据库连接</el-button>
    <el-button text @click="store.showAbout = true">关于</el-button>
  </div>
</template>

<style scoped>
.sf-nav {
  border-bottom: none;
  margin-left: 8px;
}
</style>
