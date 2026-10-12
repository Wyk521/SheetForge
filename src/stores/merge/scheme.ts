import { ref } from "vue";
import { invoke } from "../../native";
import { open, save } from "@tauri-apps/plugin-dialog";
import { ElMessage } from "element-plus";
import type { AppSettings, MergeOptions, SourceTable } from "../../types";
import type { MergeContext } from "./context";
export function createSchemeState() {
  return { settings: ref<AppSettings | null>(null) };
}
/** Scheme persistence owns settings and file I/O; task coordinates workspace changes. */
export function createSchemeActions({ settings }: Pick<MergeContext, "settings">) {
  async function persistScheme(tables: SourceTable[], options: MergeOptions) {
    const path = await save({
      title: "保存合并方案",
      defaultPath: "合并方案.json",
      filters: [{ name: "表格合并方案", extensions: ["json"] }],
    });
    if (!path) return null;
    try {
      await invoke("save_scheme", { path: String(path), tables, options });
      ElMessage.success("方案已保存");
      return String(path);
    } catch (error) {
      ElMessage.error(`保存方案失败：${error}`);
      return null;
    }
  }
  async function chooseScheme() {
    const path = await open({
      title: "打开合并方案",
      multiple: false,
      filters: [{ name: "表格合并方案", extensions: ["json"] }],
    });
    return path ? String(path) : null;
  }
  function readScheme(path: string) {
    return invoke("open_scheme", { path });
  }
  async function loadSettings() {
    const state = await invoke("get_state");
    settings.value = state.settings;
  }
  return { persistScheme, chooseScheme, readScheme, loadSettings };
}
