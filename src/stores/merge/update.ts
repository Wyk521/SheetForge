import { ref } from "vue";
import { invoke } from "../../native";
import { ElMessage } from "element-plus";
import { type UpdateResult } from "../../types";
import type { MergeContext } from "./context";
import type { NativeOn } from "../../native";
export function createUpdateState() {
  const updateText = ref("检查更新");
  const updateAvailable = ref<UpdateResult | null>(null);
  const updateDialogVisible = ref(false);
  const updateChecking = ref(false);
  const updateDownloading = ref(false);
  const updateProgress = ref(0);
  const updateStatus = ref("");
  return {
    updateText,
    updateAvailable,
    updateDialogVisible,
    updateChecking,
    updateDownloading,
    updateProgress,
    updateStatus,
  };
}
export function createUpdateActions(
  context: Pick<
    MergeContext,
    | "updateText"
    | "updateAvailable"
    | "updateDialogVisible"
    | "updateChecking"
    | "updateDownloading"
    | "updateProgress"
    | "updateStatus"
  >,
) {
  const {
    updateText,
    updateAvailable,
    updateDialogVisible,
    updateChecking,
    updateDownloading,
    updateProgress,
    updateStatus,
  } = context;
  let silentUpdateCheck = false;
  async function checkUpdate(silent = false) {
    if (updateChecking.value) return;
    silentUpdateCheck = silent;
    updateChecking.value = true;
    updateText.value = "正在检查…";
    try {
      await invoke("check_update");
    } catch (error) {
      updateChecking.value = false;
      updateText.value = "检查更新";
      if (!silent) ElMessage.error(`检查更新失败：${error}`);
    }
  }
  async function installUpdate() {
    if (!updateAvailable.value || updateDownloading.value) return;
    updateDownloading.value = true;
    updateProgress.value = 0;
    updateStatus.value = "正在下载…";
    try {
      await invoke("install_update", { update: updateAvailable.value });
    } catch (error) {
      updateDownloading.value = false;
      updateStatus.value = "";
      ElMessage.error(`更新失败：${error}`);
    }
  }
  async function registerEvents(on: NativeOn) {
    await on("update-result", (e) => {
      updateChecking.value = false;
      if (e.payload.newer) {
        updateText.value = `发现 v${e.payload.version}`;
        updateAvailable.value = e.payload;
        updateDialogVisible.value = true;
      } else {
        updateText.value = "已是最新版本";
        if (!silentUpdateCheck) ElMessage.success("已是最新版本");
      }
      silentUpdateCheck = false;
    });
    await on("update-download-progress", (e) => {
      updateStatus.value = "正在下载…";
      updateProgress.value =
        e.payload.total > 0
          ? Math.min(
              100,
              Math.round((e.payload.downloaded / e.payload.total) * 100),
            )
          : 0;
    });
    await on("update-installing", () => {
      updateProgress.value = 100;
      updateStatus.value = "正在重启…";
    });
    await on("update-failed", (e) => {
      const wasDownloading = updateDownloading.value;
      updateChecking.value = false;
      updateDownloading.value = false;
      updateProgress.value = 0;
      updateStatus.value = "";
      updateText.value =
        wasDownloading && updateAvailable.value
          ? `发现 v${updateAvailable.value.version}`
          : "检查失败，重试";
      if (wasDownloading || !silentUpdateCheck) {
        ElMessage.error(
          `${wasDownloading ? "更新" : "检查更新"}失败：${e.payload.message}`,
        );
      }
      silentUpdateCheck = false;
    });
  }
  return { checkUpdate, installUpdate, registerEvents };
}
