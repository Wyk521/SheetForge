import type { NativeEvents } from "../src/native";
import type { AppSettings, MergeScheme } from "../src/types";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import {
  defaultOptions,
  type SourceTable,
  type PreviewTable,
  type MergedPreview,
} from "../src/types";

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  success: vi.fn(),
  confirm: vi.fn(),
  handlers: new Map<string, Set<(event: { payload: unknown }) => unknown>>(),
  listen: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: native.listen }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: native.open,
  save: native.save,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));
vi.mock("element-plus", () => ({
  ElMessage: {
    error: native.error,
    warning: native.warning,
    success: native.success,
  },
  ElMessageBox: { confirm: native.confirm },
}));
import { headerKey, useMergeStore } from "../src/stores/merge";

function table(path = "C:\\数据\\一.xlsx", sheet_name = "Sheet1"): SourceTable {
  return {
    path,
    sheet_name,
    kind: "Workbook",
    enabled: true,
    header_row: 1,
    header_rows: 1,
    suggested_header_row: 1,
    estimated_rows: 2,
    headers: ["姓名", "金额"],
    mappings: ["姓名", "金额"].map((name, source_index) => ({
      source_index,
      source_name: name,
      target_name: name,
      enabled: true,
      transform: "None",
    })),
  };
}
function groupId(store: ReturnType<typeof useMergeStore>, name: string, target?: string) {
  const group = store.fieldGroups.find((group) => headerKey(group.key) === headerKey(name)
    && (target === undefined || headerKey(group.uniformTarget ?? "") === headerKey(target)));
  expect(group).toBeDefined();
  return group!.id;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
async function emit<K extends keyof NativeEvents>(
  name: K,
  payload: NativeEvents[K],
) {
  await Promise.all(
    [...(native.handlers.get(name) ?? [])].map((handler) =>
      handler({ payload }),
    ),
  );
}
function called(name: string) {
  return native.invoke.mock.calls.filter(([command]) => command === name);
}
const settings: AppSettings = {
  output_directory: "",
  recent_folders: [],
  recent_schemes: [],
  window_maximized: false,
  check_updates: false,
};
const profile = {
  host: "localhost",
  port: 5432,
  database: "test",
  user: "test",
  sslmode: "prefer",
};

beforeEach(() => {
  setActivePinia(createPinia());
  vi.resetAllMocks();
  native.handlers.clear();
  native.confirm.mockResolvedValue("confirm");
  native.listen.mockImplementation(async (name, handler) => {
    const handlers = native.handlers.get(name) ?? new Set();
    handlers.add(handler);
    native.handlers.set(name, handlers);
    return () => handlers.delete(handler);
  });
  native.invoke.mockImplementation(async (command, args) => {
    if (command === "get_plan")
      return {
        headers: args.tables.some((t: SourceTable) => t.enabled)
          ? ["姓名", "金额"]
          : [],
      };
    if (command === "get_state") return { settings };
    if (command === "get_database_profiles")
      return { profiles: { local: profile }, config_path: "test.toml" };
    if (command === "path_exists") return false;
    return null;
  });
});

describe("字段规则和来源", () => {
  it("同名字段按来源拆开后，整组改名、启用、清洗和恢复只作用于当前组", () => {
    const store = useMergeStore();
    store.sources = [table("一.xlsx"), table("二.xlsx"), table("三.xlsx"),
      { ...table("停用.xlsx"), enabled: false }];
    const original = store.fieldGroups.find((group) => group.key === "姓名")!;
    store.setSourceFieldTargets([original.tables[1].id], "收货人");
    const groups = store.fieldGroups.filter((group) => group.key === "姓名");
    expect(groups.map((group) => [group.uniformTarget, group.count])).toEqual(
      expect.arrayContaining([["姓名", 2], ["收货人", 1]]),
    );
    store.setFieldTarget(groupId(store, "姓名", "姓名"), "客户");
    store.setFieldTransform(groupId(store, "姓名", "客户"), "Trim");
    store.setFieldEnabled(groupId(store, "姓名", "客户"), false);
    expect(store.sources[1].mappings[0]).toMatchObject({
      target_name: "收货人", transform: "None", enabled: true,
    });
    store.resetField(groupId(store, "姓名", "收货人"));
    expect(store.sources.map((source) => source.mappings[0].target_name)).toEqual([
      "客户", "姓名", "客户", "姓名",
    ]);
    expect(store.sources[0].mappings[0]).toMatchObject({ transform: "Trim", enabled: false });
    expect(store.sources[3].mappings[0]).toMatchObject({ transform: "None", enabled: true });
  });
  it("拆分支持多选，重新使用同一个目标名后归为同组", () => {
    const store = useMergeStore();
    store.sources = [table("一.xlsx"), table("二.xlsx"), table("三.xlsx")];
    const group = store.fieldGroups.find((group) => group.key === "姓名")!;
    const ids = group.tables.slice(1).map((source) => source.id);
    store.setSourceFieldTargets(ids, " 收货人 ");
    expect(store.fieldGroups.find((group) => group.uniformTarget === "收货人")?.count).toBe(2);
    store.setSourceFieldTargets(ids, "姓名");
    expect(store.fieldGroups.filter((group) => group.key === "姓名")).toHaveLength(1);
    expect(store.fieldGroups.find((group) => group.key === "姓名")?.count).toBe(3);
    store.setSourceFieldTargets(ids, "  ");
    expect(store.sources.every((source) => source.mappings[0].target_name === "姓名")).toBe(true);
  });
  it("按来源修改不依赖列表下标，追加排序或移除来源后不会改错表", () => {
    const store = useMergeStore();
    store.sources = [table("B.xlsx"), table("C.xlsx")];
    const source = store.fieldGroups.find((group) => group.key === "姓名")!.tables[1];
    store.sources = [table("A.xlsx"), ...store.sources];
    store.setSourceFieldTargets([source.id], "客户");
    expect(store.sources.map((table) => table.mappings[0].target_name)).toEqual(["姓名", "姓名", "客户"]);
    store.sources.pop();
    store.setSourceFieldTargets([source.id], "不应写入");
    expect(store.sources.every((table) => table.mappings[0].target_name === "姓名")).toBe(true);
  });
  it("重复列名用列位置区分，同表计数不会重复", () => {
    const store = useMergeStore();
    const source = table();
    source.headers[1] = "姓名";
    source.mappings[1] = { ...source.mappings[0], source_index: 1 };
    store.sources = [source];
    const group = store.fieldGroups[0];
    expect(group.count).toBe(1);
    expect(group.tables[0].id).not.toBe(group.tables[1].id);
    store.setSourceFieldTargets([group.tables[1].id], "收货人");
    expect(store.sources[0].mappings.map((mapping) => mapping.target_name)).toEqual(["姓名", "收货人"]);
  });
  it("部分来源改名保留原输出列，重新归并不重复或移动已有目标列", () => {
    const store = useMergeStore();
    store.sources = [table("一.xlsx"), table("二.xlsx")];
    store.options.output_order = ["姓名", "金额", "来源文件"];
    const source = store.fieldGroups.find((group) => group.key === "姓名")!.tables[1];
    store.setSourceFieldTargets([source.id], "客户");
    expect(store.options.output_order).toEqual(["姓名", "客户", "金额", "来源文件"]);
    store.setFieldTarget(groupId(store, "姓名", "姓名"), "客户");
    expect(store.options.output_order).toEqual(["客户", "金额", "来源文件"]);
  });
  it("不同源字段共用目标列时，改动其中一组仍保留另一组的输出位置", () => {
    const store = useMergeStore();
    store.sources = [table()];
    store.sources[0].mappings[1].target_name = "姓名";
    store.options.output_order = ["姓名", "已有目标", "来源文件"];
    store.setFieldTarget(groupId(store, "姓名"), "已有目标");
    expect(store.options.output_order).toEqual(["姓名", "已有目标", "来源文件"]);
    expect(store.sources[0].mappings[1].target_name).toBe("姓名");
  });
  it("移除确认期间追加并排序来源后仍移除原来选中的 Sheet", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.sources = [table("B.xlsx"), table("C.xlsx"), table("C.xlsx", "Sheet2")];
    const confirmation = deferred<string>();
    native.confirm.mockReturnValueOnce(confirmation.promise);
    const removal = store.removeSource(1);
    native.open.mockResolvedValueOnce(["A.xlsx"]);
    await store.chooseFiles();
    await emit("scan-finished", { tables: [table("A.xlsx")], warnings: [] });
    confirmation.resolve("confirm");
    await removal;
    expect(store.sources.map((source) => [source.path, source.sheet_name])).toEqual([
      ["A.xlsx", "Sheet1"], ["B.xlsx", "Sheet1"], ["C.xlsx", "Sheet2"],
    ]);
  });
  it("表头换行归一化保留普通空格语义", () => {
    expect(headerKey(" A\r\nB ")).toBe("ab");
    expect(headerKey("A B")).toBe("a b");
  });
  it("字段修改只作用于启用表，同步维护输出顺序", () => {
    const store = useMergeStore();
    store.sources = [table(), { ...table("二.xlsx"), enabled: false }];
    store.options.output_order = ["姓名", "金额"];
    store.setFieldTarget(groupId(store, "姓名"), " 客户 ");
    store.setFieldTransform(groupId(store, "姓名"), "Trim");
    expect(store.sources[0].mappings[0]).toMatchObject({
      target_name: "客户",
      transform: "Trim",
    });
    expect(store.sources[1].mappings[0].target_name).toBe("姓名");
    expect(store.options.output_order).toEqual(["客户", "金额"]);
    store.resetField(groupId(store, "姓名"));
    expect(store.sources[0].mappings[0]).toMatchObject({
      target_name: "姓名",
      transform: "None",
      enabled: true,
    });
    expect(store.options.output_order).toEqual(["姓名", "金额"]);
  });
  it("筛选全选仅启用当前命中的表", () => {
    const store = useMergeStore();
    store.sources = [table("一.xlsx"), table("二.xlsx")];
    store.sourceSearch = "二";
    store.selectAll(true, true);
    expect(store.sources.map((source) => source.enabled)).toEqual([
      false,
      true,
    ]);
    expect(store.visibleEnabledCount).toBe(1);
  });
  it("字段启用和按索引、名称排序维护真实输出顺序及边界", async () => {
    const store = useMergeStore();
    store.sources = [table(), { ...table("停用.xlsx"), enabled: false }];
    native.invoke.mockImplementation(async (command, args) => {
      if (command === "get_plan")
        return { headers: args.options.output_order.length
          ? args.options.output_order : ["姓名", "金额", "来源文件"] };
    });
    await store.refreshPlan();
    store.moveOutputColumn(0, 1);
    await vi.waitFor(() => expect(store.planHeaders).toEqual(["金额", "姓名", "来源文件"]));
    store.moveOutputColumnByName(" 姓名\n", 1);
    await vi.waitFor(() => expect(store.planHeaders).toEqual(["金额", "来源文件", "姓名"]));
    store.moveOutputColumn(2, 10);
    store.moveOutputColumnByName("不存在", -1);
    store.moveOutputColumn(-1, 1);
    expect(store.options.output_order).toEqual(["金额", "来源文件", "姓名"]);
    store.setFieldEnabled(groupId(store, "姓名"), false);
    expect(store.sources.map((source) => source.mappings[0].enabled)).toEqual([false, true]);
    store.setFieldEnabled(groupId(store, "姓名"), true);
    expect(store.sources[0].mappings[0].enabled).toBe(true);
  });
  it("追加扫描去重并保留已有映射和顺序", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.sources = [table()];
    store.sources[0].mappings[0].target_name = "客户";
    store.options.output_order = ["金额", "客户"];
    await store.scanFiles(["二.xlsx"]);
    await emit("scan-finished", {
      tables: [table(), table("二.xlsx")],
      warnings: ["提示"],
    });
    expect(store.sources).toHaveLength(2);
    expect(
      store.sources.find((source) => source.path.includes("一.xlsx"))
        ?.mappings[0].target_name,
    ).toBe("客户");
    expect(store.options.output_order).toEqual(["金额", "客户"]);
    expect(store.phase).toBe("ready");
    expect(store.warnings).toEqual(["提示"]);
  });
  it("重载以来源标识回写，避免列表下标变化覆盖错误来源", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.sources = [table("二.xlsx"), table("一.xlsx")];
    const updated = { ...table("一.xlsx"), header_row: 3 };
    await emit("tables-reloaded", {
      tables: [{ index: 0, table: updated }],
      failures: ["二.xlsx 失败"],
    });
    expect(store.sources[0].header_row).toBe(1);
    expect(store.sources[1].header_row).toBe(3);
    expect(store.phase).toBe("ready");
    expect(native.warning).toHaveBeenCalled();
  });
});

describe("预览请求", () => {
  it("重复打开同一源表直接显示缓存，字段映射变化不重读原始数据", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    const preview = { headers: ["姓名", "金额"], rows: [["张三", "100"]] };
    native.invoke.mockResolvedValueOnce(preview);
    await store.showSourcePreview(0);
    store.closeSourcePreview();
    store.setFieldTarget(groupId(store, "姓名"), "客户");
    await store.showSourcePreview(0);
    expect(called("preview_source")).toHaveLength(1);
    expect(store.sourcePreview).toEqual(preview);
    expect(store.sourcePreviewVisible).toBe(true);
    expect(store.sourcePreviewLoading).toBe(false);
  });
  it("关闭后在同一读取结束前再次打开会共用请求", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    const result = deferred<PreviewTable>();
    native.invoke.mockReturnValueOnce(result.promise);
    const first = store.showSourcePreview(0);
    store.closeSourcePreview();
    const second = store.showSourcePreview(0);
    expect(called("preview_source")).toHaveLength(1);
    result.resolve({ headers: ["姓名"], rows: [["张三"]] });
    await Promise.all([first, second]);
    expect(store.sourcePreview?.rows).toEqual([["张三"]]);
    expect(store.sourcePreviewVisible).toBe(true);
  });
  it("手动刷新会重新读取，旧请求不会覆盖刷新结果或缓存", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    const old = deferred<PreviewTable>();
    const latest = { headers: ["姓名"], rows: [["新内容"]] };
    native.invoke.mockReturnValueOnce(old.promise).mockResolvedValueOnce(latest);
    const pending = store.showSourcePreview(0);
    await store.refreshSourcePreview();
    old.resolve({ headers: ["姓名"], rows: [["旧内容"]] });
    await pending;
    store.closeSourcePreview();
    await store.showSourcePreview(0);
    expect(called("preview_source")).toHaveLength(2);
    expect(store.sourcePreview).toEqual(latest);
  });
  it("快速切换来源最多读取两张表，尚未开始的旧预览让位给当前来源", async () => {
    const store = useMergeStore();
    store.sources = [0, 1, 2, 3].map(index => table(`${index}.xlsx`));
    const reads = [0, 1, 2, 3].map(() => deferred<PreviewTable>());
    native.invoke.mockImplementation((command, args) => command === "preview_source"
      ? reads[Number(args.table.path[0])].promise : Promise.resolve({ headers: [] }));
    const requests = [0, 1, 2, 3].map(index => store.showSourcePreview(index));
    expect(called("preview_source")).toHaveLength(2);
    reads[0].resolve({ headers: ["姓名"], rows: [["旧"]] });
    await requests[0];
    await Promise.resolve();
    expect(called("preview_source").map(call => call[1].table.path)).toEqual(["0.xlsx", "1.xlsx", "3.xlsx"]);
    reads[3].resolve({ headers: ["姓名"], rows: [["最新"]] });
    reads[1].resolve({ headers: ["姓名"], rows: [["旧二"]] });
    await Promise.all(requests);
    expect(store.sourcePreview?.rows).toEqual([["最新"]]);
    expect(native.error).not.toHaveBeenCalled();
  });
  it("切回仍在读取的来源复用同一请求，并丢弃未开始的其他来源", async () => {
    const store = useMergeStore();
    store.sources = [0, 1, 2].map(index => table(`${index}.xlsx`));
    const reads = [deferred<PreviewTable>(), deferred<PreviewTable>()];
    native.invoke.mockImplementation((command, args) => command === "preview_source"
      ? reads[Number(args.table.path[0])].promise : Promise.resolve({ headers: [] }));
    const requests = [0, 1, 2, 0].map(index => store.showSourcePreview(index));
    reads[1].resolve({ headers: ["姓名"], rows: [["二"]] });
    await requests[1];
    reads[0].resolve({ headers: ["姓名"], rows: [["一"]] });
    await Promise.all(requests);
    expect(called("preview_source")).toHaveLength(2);
    expect(store.sourcePreview?.rows).toEqual([["一"]]);
    expect(native.error).not.toHaveBeenCalled();
  });
  it("重新导入同一路径或改变表头后重新读取预览", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    native.invoke.mockResolvedValue({ headers: ["姓名"], rows: [["内容"]] });
    await store.showSourcePreview(0);
    store.closeSourcePreview();
    store.sources = [table()];
    await store.showSourcePreview(0);
    store.sources[0].header_row = 2;
    await store.showSourcePreview(0);
    expect(called("preview_source")).toHaveLength(3);
    expect(called("preview_source")[2][1].table.header_row).toBe(2);
  });
  it("预览读取失败后可以重新打开重试", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    native.invoke.mockRejectedValueOnce(new Error("文件不可用"));
    await store.showSourcePreview(0);
    expect(store.sourcePreviewVisible).toBe(false);
    native.invoke.mockResolvedValueOnce({ headers: ["姓名"], rows: [["已恢复"]] });
    await store.showSourcePreview(0);
    expect(called("preview_source")).toHaveLength(2);
    expect(store.sourcePreview?.rows).toEqual([["已恢复"]]);
  });
  it("源表预览忽略较早返回的结果", async () => {
    const store = useMergeStore();
    store.sources = [table(), table("二.xlsx")];
    const old = deferred<PreviewTable>();
    const latest = { headers: ["新"], rows: [["新值"]] };
    native.invoke
      .mockImplementationOnce(() => old.promise)
      .mockResolvedValueOnce(latest);
    const pending = store.showSourcePreview(0);
    await store.showSourcePreview(1);
    old.resolve({ headers: ["旧"], rows: [["旧值"]] });
    await pending;
    expect(store.sourcePreview).toEqual(latest);
    expect(store.sourcePreviewLoading).toBe(false);
  });
  it("关闭预览后不被未完成的请求重新打开", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    const result = deferred<PreviewTable>();
    native.invoke.mockImplementationOnce(() => result.promise);
    const pending = store.showSourcePreview(0);
    store.closeSourcePreview();
    result.resolve({ headers: [], rows: [] });
    await pending;
    expect(store.sourcePreviewVisible).toBe(false);
    expect(store.sourcePreview).toBeNull();
  });
  it("结果预览保留每来源五行抽样参数并忽略过期结果", async () => {
    const store = useMergeStore();
    const old = deferred<MergedPreview>();
    const latest = { headers: ["新"], groups: [] };
    native.invoke
      .mockImplementationOnce(() => old.promise)
      .mockResolvedValueOnce(latest);
    const pending = store.showMergedPreview();
    await store.showMergedPreview();
    old.resolve({ headers: ["旧"], groups: [] });
    await pending;
    expect(store.preview).toEqual(latest);
    expect(called("preview_merged")[0][1].limit).toBe(5);
  });
});

describe("处理任务", () => {
  it("扫描失败详情保留在检查报告中，修改规则和重新检查不会丢失", async () => {
    const store = useMergeStore();
    await store.initEvents();
    await store.scanFiles(["正常.csv", "损坏.xlsx"]);
    await emit("scan-finished", {
      tables: [table("正常.csv")], warnings: ["损坏.xlsx：无法读取工作簿"],
    });
    expect(store.progressLabel).toContain("1 个扫描提醒");
    expect(store.reportIssues).toContainEqual({
      level: "Warning", title: "扫描提醒", detail: "损坏.xlsx：无法读取工作簿",
    });
    store.setAdvanced({ deduplicate: true });
    await store.runPreflight();
    await emit("preflight-done", { issues: [], continues_merge: false });
    expect(store.reportIssues).toHaveLength(1);
    expect(store.reportIssues[0].detail).toContain("损坏.xlsx");
    expect(store.progressLabel).toBe("检查完成：0 个错误，1 个提醒");
  });
  it("后台完成先于命令确认返回时仍保持完成状态", async () => {
    const store = useMergeStore();
    await store.initEvents();
    await store.runPreflight(true);
    store.outputPath = "结果.xlsx";
    native.invoke.mockImplementation(async (command) => {
      if (command === "path_exists") return false;
      if (command === "start_merge")
        await emit("merge-finished", {
          output: "结果.xlsx",
          rows: 0,
          sheets: 1,
          truncated_cells: 0,
        });
    });
    await emit("preflight-done", { issues: [], continues_merge: true });
    expect(store.phase).toBe("ready");
    expect(store.progressLabel).toBe("合并完成");
  });
  it("启动失败可重新检查并重试，取消请求失败保留运行状态", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.outputDestination = "postgres";
    await store.runPreflight(true);
    native.invoke.mockRejectedValueOnce(new Error("启动失败"));
    await emit("preflight-done", { issues: [], continues_merge: true });
    expect(store.phase).toBe("ready");
    expect(store.progressLabel).toBe("");
    await store.runPreflight();
    expect(store.phase).toBe("checking");
    store.phase = "merging";
    store.progressLabel = "正在导入";
    native.invoke.mockRejectedValueOnce(new Error("无法取消"));
    await expect(store.cancelMerge()).resolves.toBeUndefined();
    expect(store.phase).toBe("merging");
    expect(store.progressLabel).toBe("正在导入");
  });
  it("处理期间拖入文件不启动第二个扫描任务", async () => {
    const store = useMergeStore();
    store.phase = "merging";
    await store.scanFiles(["二.xlsx"]);
    expect(called("scan_files")).toHaveLength(0);
    expect(store.phase).toBe("merging");
  });
  it("预检调用被拒绝后恢复状态并显示错误", async () => {
    const store = useMergeStore();
    native.invoke.mockRejectedValueOnce(new Error("无法检查"));
    await expect(store.runPreflight()).resolves.toBeUndefined();
    expect(store.phase).toBe("ready");
    expect(native.error).toHaveBeenCalled();
  });
  it("表头重载调用失败后恢复状态", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    native.invoke.mockImplementationOnce(() => {
      throw new Error("无法重载");
    });
    await expect(
      (async () => store.reloadTable(0, 2, 1))(),
    ).resolves.toBeUndefined();
    expect(store.phase).toBe("ready");
    expect(native.error).toHaveBeenCalled();
  });
  it("整本工作簿表头重载失败后保留来源和映射并可重试", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.sources = [table(), table(table().path, "Sheet2")];
    store.sources[0].header_row = 3;
    store.sources[0].header_rows = 2;
    store.sources[0].mappings[0].target_name = "客户";
    const before = structuredClone(JSON.parse(JSON.stringify(store.sources)));
    native.invoke.mockRejectedValueOnce(new Error("无法重载整本工作簿"));
    await store.applyGroupHeader(table().path);
    expect(store.phase).toBe("ready");
    expect(store.progressLabel).toBe("");
    expect(store.sources).toEqual(before);
    expect(native.error).toHaveBeenCalledWith("Error: 无法重载整本工作簿");
    await store.applyGroupHeader(table().path);
    expect(called("reload_group")[1][1]).toMatchObject({headerRow: 3, headerRows: 2});
    expect(store.phase).toBe("scanning");
    await emit("tables-reloaded", { tables: before.map((source: SourceTable, index: number) => ({index, table: {...source, header_row: 3, header_rows: 2}})), failures: [] });
    expect(store.phase).toBe("ready");
    expect(store.sources.every((source) => source.header_row === 3 && source.header_rows === 2)).toBe(true);
  });
  it("预检错误阻止实际写出", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.outputPath = "结果.xlsx";
    await store.startMerge();
    expect(store.phase).toBe("checking");
    await emit("preflight-done", {
      issues: [{ level: "Error", title: "没有字段", detail: "请选择字段" }],
      continues_merge: true,
    });
    expect(called("start_merge")).toHaveLength(0);
    expect(store.phase).toBe("ready");
  });
  it("取消覆盖确认不启动合并", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.outputPath = "结果.xlsx";
    await store.startMerge();
    native.invoke.mockResolvedValueOnce(true);
    native.confirm.mockRejectedValueOnce("cancel");
    await emit("preflight-done", { issues: [], continues_merge: true });
    expect(called("start_merge")).toHaveLength(0);
    expect(store.phase).toBe("ready");
  });
  it("输出路径去除首尾空白后再检查覆盖并写入同一位置", async () => {
    const store = useMergeStore();
    await store.initEvents();
    const output = "C:\\数据\\结果.xlsx";
    store.outputPath = ` ${output} `;
    const original = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation(async (command, args) =>
      command === "path_exists" ? args.path === output : original(command, args),
    );
    await store.startMerge();
    await emit("preflight-done", { issues: [], continues_merge: true });
    expect(called("path_exists")[0][1].path).toBe(output);
    expect(native.confirm).toHaveBeenCalledWith(
      `${output} 已存在，是否覆盖？`, "覆盖已有文件", { type: "warning" },
    );
    expect(called("start_merge")[0][1].output).toBe(output);
  });
  it("数据库清空须确认，开始命令携带本次目标", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.outputDestination = "postgres";
    store.databaseImport = {
      ...store.databaseImport,
      profile_name: "local",
      table: "结果",
      if_exists: "truncate",
    };
    await store.runPreflight(true);
    await emit("preflight-done", { issues: [], continues_merge: true });
    expect(native.confirm).toHaveBeenCalled();
    expect(called("start_database_import")[0][1].request.table).toBe("结果");
    expect(store.phase).toBe("merging");
  });
  it("检查期间改动规则会阻止写出，重新检查后才使用新规则", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.sources = [table()];
    store.outputPath = "结果.xlsx";
    store.options.filter_column = "姓名";
    store.options.filter_text = "Alice";
    await store.startMerge();
    store.setAdvanced({ filter_column: "不存在的列" });
    await emit("preflight-done", { issues: [], continues_merge: true });
    expect(called("start_merge")).toHaveLength(0);
    expect(store.phase).toBe("ready");
    expect(store.checkRan).toBe(false);
    expect(native.warning).toHaveBeenCalledWith("数据源或规则已修改，请重新检查后再开始");
    store.setAdvanced({ filter_column: "姓名", filter_text: "Bob" });
    await store.startMerge();
    await emit("preflight-done", { issues: [], continues_merge: true });
    expect(called("start_merge")[0][1].options.filter_text).toBe("Bob");
  });
  it("覆盖确认期间修改规则也会作废检查", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.outputPath = "结果.xlsx";
    await store.startMerge();
    native.invoke.mockResolvedValueOnce(true);
    const confirmation = deferred<string>();
    native.confirm.mockReturnValueOnce(confirmation.promise);
    const pending = emit("preflight-done", { issues: [], continues_merge: true });
    await vi.waitFor(() => expect(native.confirm).toHaveBeenCalled());
    store.setAdvanced({ deduplicate: true });
    confirmation.resolve("confirm");
    await pending;
    expect(called("start_merge")).toHaveLength(0);
    expect(store.checkRan).toBe(false);
  });
  it("取消后等待后台结束事件才解除忙碌", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.phase = "merging";
    await store.cancelMerge();
    expect(called("cancel_merge")).toHaveLength(1);
    expect(store.busy).toBe(true);
    await emit("merge-cancelled", null);
    expect(store.busy).toBe(false);
    expect(store.progressLabel).toBe("已取消");
  });
  it("后台失败恢复可操作状态", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.phase = "merging";
    await emit("merge-failed", { message: "无法写入" });
    expect(store.busy).toBe(false);
    expect(native.error).toHaveBeenCalledWith("合并失败：无法写入");
  });
});

describe("异步状态和事件生命周期", () => {
  it("部分监听注册失败会释放已注册项并允许重试", async () => {
    const store = useMergeStore();
    const original = native.listen.getMockImplementation()!;
    native.listen
      .mockImplementationOnce(original)
      .mockRejectedValueOnce(new Error("监听失败"));
    await expect(store.initEvents()).rejects.toThrow("监听失败");
    expect(
      [...native.handlers.values()].every((handlers) => handlers.size === 0),
    ).toBe(true);
    await store.initEvents();
    expect(native.handlers.get("merge-finished")?.size).toBe(1);
    store.disposeEvents();
    await store.initEvents();
    expect(native.handlers.get("merge-finished")?.size).toBe(1);
  });
  it("销毁后才完成的监听注册也会被释放", async () => {
    const store = useMergeStore();
    const late = deferred<() => void>();
    const release = vi.fn();
    native.listen.mockImplementationOnce(() => late.promise);
    const pending = store.initEvents();
    store.$dispose();
    late.resolve(release);
    await pending;
    expect(release).toHaveBeenCalledTimes(1);
    expect(native.listen).toHaveBeenCalledTimes(1);
  });
  it("输出计划不会被较早返回的响应覆盖", async () => {
    const store = useMergeStore();
    const old = deferred<{ headers: string[] }>();
    native.invoke
      .mockImplementationOnce(() => old.promise)
      .mockResolvedValueOnce({ headers: ["最新"] });
    const pending = store.refreshPlan();
    await store.refreshPlan();
    old.resolve({ headers: ["过期"] });
    await pending;
    expect(store.planHeaders).toEqual(["最新"]);
  });
  it("同一轮连续规则编辑合并为一次计划请求，旧响应立即失效", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    const old = deferred<{ headers: string[] }>();
    native.invoke.mockImplementationOnce(() => old.promise).mockResolvedValue({ headers: ["当前"] });
    const pending = store.refreshPlan();
    for (let index = 0; index < 100; index++) store.setAdvanced({ include_source_file: index % 2 === 0 });
    old.resolve({ headers: ["旧"] });
    await pending;
    await Promise.resolve();
    expect(called("get_plan")).toHaveLength(2);
    expect(called("get_plan")[1][1].options.include_source_file).toBe(false);
    expect(store.planHeaders).toEqual(["当前"]);
  });
  it("来源移动、停用和名称变更后字段来源仍指向当前表", () => {
    const store = useMergeStore();
    store.sources = [table("一.xlsx"), table("二.xlsx")];
    expect(store.fieldGroups[0].count).toBe(2);
    store.sources.reverse();
    store.sources[0].sheet_name = "新工作表";
    store.sources[1].enabled = false;
    const group = store.fieldGroups.find(group => group.key === "姓名")!;
    expect(group.tables).toHaveLength(1);
    expect(group.tables[0]).toMatchObject({ index: 0, name: "二.xlsx  /  新工作表" });
    store.setSourceFieldTargets([group.tables[0].id], "独立姓名");
    expect(store.sources[0].mappings[0].target_name).toBe("独立姓名");
    expect(store.sources[1].mappings[0].target_name).toBe("姓名");
  });
  it("筛选和去重不改变表头，编辑时只作废结果而不重发整批计划", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    await store.refreshPlan();
    store.preview = { headers: ["旧"], groups: [] };
    store.checkRan = true;
    for (let index = 0; index < 100; index++) store.setAdvanced({ filter_text: String(index), deduplicate: true });
    await Promise.resolve();
    expect(called("get_plan")).toHaveLength(1);
    expect(store.planHeaders).toEqual(["姓名", "金额"]);
    expect(store.preview).toBeNull();
    expect(store.checkRan).toBe(false);
  });
  it("修改规则立即作废结果预览和检查", () => {
    const store = useMergeStore();
    store.preview = { headers: ["旧"], groups: [] };
    store.checkRan = true;
    store.setAdvanced({ deduplicate: true });
    expect(store.preview).toBeNull();
    expect(store.checkRan).toBe(false);
  });
  it("并发初始化仅注册一次监听，销毁时全部释放", async () => {
    const store = useMergeStore();
    await Promise.all([store.initEvents(), store.initEvents()]);
    expect(native.handlers.get("merge-finished")?.size).toBe(1);
    store.$dispose();
    expect(
      [...native.handlers.values()].every((handlers) => handlers.size === 0),
    ).toBe(true);
  });
});

describe("数据库、方案和更新", () => {
  it("拆分字段保存再打开仍保持独立，并保留清洗、启用和输出顺序", async () => {
    const store = useMergeStore();
    store.sources = [table("一.xlsx"), table("二.xlsx")];
    store.options.output_order = ["姓名", "金额"];
    const source = store.fieldGroups.find((group) => group.key === "姓名")!.tables[1];
    store.setSourceFieldTargets([source.id], "收货人");
    store.setFieldTransform(groupId(store, "姓名", "收货人"), "Trim");
    store.setFieldEnabled(groupId(store, "姓名", "收货人"), false);
    let saved: MergeScheme;
    const original = native.invoke.getMockImplementation()!;
    native.save.mockResolvedValueOnce("拆分方案.json");
    native.invoke.mockImplementation(async (command, args) => {
      if (command === "save_scheme") {
        saved = JSON.parse(JSON.stringify({ format_version: 1, name: "拆分方案", tables: args.tables, options: args.options }));
        return;
      }
      if (command === "open_scheme") return saved;
      return original(command, args);
    });
    await store.saveScheme();
    store.sources = [];
    store.options = defaultOptions();
    await store.openSchemeByPath("拆分方案.json");
    expect(store.fieldGroups.filter((group) => group.key === "姓名")).toHaveLength(2);
    expect(store.sources[1].mappings[0]).toMatchObject({ target_name: "收货人", transform: "Trim", enabled: false });
    expect(store.options.output_order).toEqual(["姓名", "收货人", "金额"]);
    store.setFieldTarget(groupId(store, "姓名", "姓名"), "客户");
    expect(store.sources[1].mappings[0].target_name).toBe("收货人");
  });
  it("方案保存再打开完整恢复来源、映射和所有规则", async () => {
    const store = useMergeStore();
    store.sources = [table(), {...table("停用.csv", "数据"), kind: {Csv: {delimiter: 9}}, enabled: false}];
    Object.assign(store.sources[0], {header_row: 4, header_rows: 2, estimated_rows: 42});
    Object.assign(store.sources[0].mappings[0], {target_name: "客户", transform: "Trim"});
    store.sources[0].mappings[1].enabled = false;
    store.options = {mode: "Intersection", include_source_file: true, include_source_sheet: true,
      output_order: ["来源文件", "客户"], deduplicate: true, filter_column: "客户", filter_text: "测试", filter_exclude: true};
    const expected = JSON.parse(JSON.stringify({tables: store.sources, options: store.options}));
    let saved: MergeScheme;
    const original = native.invoke.getMockImplementation()!;
    native.save.mockResolvedValueOnce("完整方案.json");
    native.invoke.mockImplementation(async (command, args) => {
      if (command === "save_scheme") {
        saved = JSON.parse(JSON.stringify({format_version: 1, name: "完整方案", tables: args.tables, options: args.options}));
        return;
      }
      if (command === "open_scheme") return saved;
      return original(command, args);
    });
    await store.saveScheme();
    expect(called("save_scheme")[0][1]).toEqual({path: "完整方案.json", ...expected});
    store.sources = [];
    store.options = defaultOptions();
    store.preview = {headers: ["旧"], groups: []};
    store.sourcePreviewVisible = true;
    store.checkRan = true;
    await store.openSchemeByPath("完整方案.json");
    expect(store.sources).toEqual(expected.tables);
    expect(store.options).toEqual(expected.options);
    expect(store.preview).toBeNull();
    expect(store.sourcePreviewVisible).toBe(false);
    expect(store.checkRan).toBe(false);
    expect(store.inputLabel).toBe("已打开方案：完整方案.json");
    expect(store.phase).toBe("ready");
  });
  it("方案打开失败保留当前工作区并解除忙碌", async () => {
    const store = useMergeStore();
    store.sources = [table()];
    store.options.deduplicate = true;
    native.invoke.mockRejectedValueOnce(new Error("方案已损坏"));
    await store.openSchemeByPath("坏方案.json");
    expect(store.sources).toEqual([table()]);
    expect(store.options.deduplicate).toBe(true);
    expect(store.busy).toBe(false);
    expect(native.error).toHaveBeenCalledWith("打开方案失败：Error: 方案已损坏");
  });
  it("加载连接为失效选择选择有效替代项，空密码传 null", async () => {
    const store = useMergeStore();
    store.databaseImport.profile_name = "deleted";
    await store.loadDatabaseProfiles();
    expect(store.databaseImport.profile_name).toBe("local");
    await store.testDatabaseConnection(undefined, profile, "");
    expect(called("test_database_connection")[0][1]).toMatchObject({
      password: null,
      profileName: null,
    });
  });
  it("方案恢复替换来源和规则并清空过期预览", async () => {
    const store = useMergeStore();
    store.preview = { headers: ["旧"], groups: [] };
    const options = { ...defaultOptions(), deduplicate: true };
    const scheme: MergeScheme = {
      format_version: 1,
      name: "方案",
      tables: [table()],
      options,
    };
    native.invoke.mockResolvedValueOnce(scheme);
    await store.openSchemeByPath("方案.json");
    expect(store.sources).toHaveLength(1);
    expect(store.options.deduplicate).toBe(true);
    expect(store.preview).toBeNull();
    expect(store.phase).toBe("ready");
  });
  it("静默更新失败不弹错误，手动重试可收到结果", async () => {
    const store = useMergeStore();
    await store.initEvents();
    await store.checkUpdate(true);
    await emit("update-failed", { message: "断网" });
    expect(native.error).not.toHaveBeenCalled();
    expect(store.updateChecking).toBe(false);
    await store.checkUpdate();
    await emit("update-result", {
      version: "1.0.0",
      url: "test",
      sha256: "test",
      size: 10,
      newer: true,
    });
    expect(store.updateDialogVisible).toBe(true);
    expect(store.updateAvailable?.version).toBe("1.0.0");
  });
  it("下载失败恢复状态并保留版本供重试", async () => {
    const store = useMergeStore();
    await store.initEvents();
    store.updateAvailable = {
      version: "1.0.0",
      url: "test",
      sha256: "test",
      size: 10,
      newer: true,
    };
    await store.installUpdate();
    await emit("update-download-progress", { downloaded: 5, total: 10 });
    expect(store.updateProgress).toBe(50);
    await emit("update-failed", { message: "下载失败" });
    expect(store.updateDownloading).toBe(false);
    expect(store.updateProgress).toBe(0);
    expect(store.updateAvailable).not.toBeNull();
    await store.installUpdate();
    expect(called("install_update")).toHaveLength(2);
  });
});
