<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import TopBar from "./components/TopBar.vue";
import TitleBar from "./components/TitleBar.vue";
import BottomBar from "./components/BottomBar.vue";
import AboutDialog from "./components/AboutDialog.vue";
import DataSourceView from "./views/DataSourceView.vue";
import MergeRulesView from "./views/MergeRulesView.vue";
import PreviewView from "./views/PreviewView.vue";
import { useMergeStore } from "./stores/merge";

const store = useMergeStore();

function onKeyDown(event: KeyboardEvent) {
  if (event.ctrlKey && event.key.toLowerCase() === "o") {
    event.preventDefault();
    void store.chooseFiles();
  } else if (event.ctrlKey && event.key.toLowerCase() === "s") {
    event.preventDefault();
    void store.saveScheme();
  } else if (event.ctrlKey && event.key.toLowerCase() === "l") {
    event.preventDefault();
    void store.openScheme();
  } else if (event.key === "Escape") {
    store.showAbout = false;
  }
}

// 屏蔽 WebView 自带的浏览器右键菜单（后退/刷新/查看源代码等，容易误操作）；
// 文本框/文本域仍保留原生菜单，方便右键粘贴。
function onContextMenu(event: MouseEvent) {
  const target = event.target as HTMLElement | null;
  if (target?.closest?.("input, textarea, [contenteditable='true']")) return;
  event.preventDefault();
}

let unlistenDrop: (() => void) | undefined;

onMounted(async () => {
  await store.initEvents();
  await store.loadState();
  await store.refreshPlan();
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("contextmenu", onContextMenu);
  unlistenDrop = await getCurrentWebviewWindow().onDragDropEvent((event) => {
    if (event.payload.type === "drop") {
      void store.scanFiles(event.payload.paths);
    }
  });
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("contextmenu", onContextMenu);
  unlistenDrop?.();
});
</script>

<template>
  <div class="app-shell">
    <TitleBar />
    <TopBar />
    <div class="app-content">
      <DataSourceView v-if="store.activePage === 0" />
      <MergeRulesView v-else-if="store.activePage === 1" />
      <PreviewView v-else />
    </div>
    <BottomBar />
    <AboutDialog />
  </div>
</template>
