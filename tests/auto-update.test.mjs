import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

test("启动时从 NB 静默检查更新并显示更新窗口", async () => {
  const [app, store, dialog, updater] = await Promise.all([
    read("src/App.vue"),
    read("src/stores/merge.ts"),
    read("src/components/UpdateDialog.vue"),
    read("src-tauri/src/updater.rs"),
  ]);

  assert.match(app, /store\.checkUpdate\(true\)/);
  assert.match(app, /<UpdateDialog\s*\/>/);
  assert.match(store, /updateDialogVisible\.value = true/);
  assert.match(dialog, /立即更新/);
  assert.match(dialog, /更新后软件会自动重启/);
  assert.match(updater, /https:\/\/jp\.kun\.show:8443\/sheetforge\/latest\.json/);
});

test("便携版更新校验文件并在退出后替换重启", async () => {
  const updater = await read("src-tauri/src/updater.rs");

  assert.match(updater, /eq_ignore_ascii_case\(&update\.sha256\)/);
  assert.match(updater, /\$oldProcess \| Wait-Process -Timeout 60/);
  assert.match(updater, /Move-Item -LiteralPath \$replacement -Destination \$currentExe/);
  assert.match(updater, /Start-Process -FilePath \$currentExe/);
  assert.match(updater, /app\.exit\(0\)/);
});

test("Windows 发布只保留便携版和校验文件", async () => {
  const [workflow, tauri] = await Promise.all([
    read(".github/workflows/build-windows.yml"),
    read("src-tauri/tauri.conf.json"),
  ]);

  assert.doesNotMatch(workflow, /setup\.exe|bundle\\nsis|\*-setup/);
  assert.match(workflow, /dist\\SheetForge\.exe/);
  assert.match(workflow, /dist\\SHA256SUMS\.txt/);
  assert.equal(JSON.parse(tauri).bundle.active, false);
});
