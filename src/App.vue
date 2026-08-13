<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import TopBar from "./components/TopBar.vue";
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

let unlistenDrop: (() => void) | undefined;

onMounted(async () => {
  await store.initEvents();
  await store.loadState();
  await store.refreshPlan();
  window.addEventListener("keydown", onKeyDown);
  unlistenDrop = await getCurrentWebviewWindow().onDragDropEvent((event) => {
    if (event.payload.type === "drop") {
      void store.scanFiles(event.payload.paths);
    }
  });
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
  unlistenDrop?.();
});
</script>

<template>
  <div class="app-shell">
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
