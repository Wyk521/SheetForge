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
  it("表头换行归一化保留普通空格语义", () => {
    expect(headerKey(" A\r\nB ")).toBe("ab");
    expect(headerKey("A B")).toBe("a b");
  });
  it("字段修改只作用于启用表，同步维护输出顺序", () => {
    const store = useMergeStore();
    store.sources = [table(), { ...table("二.xlsx"), enabled: false }];
    store.options.output_order = ["姓名", "金额"];
    store.setFieldTarget("姓名", " 客户 ");
    store.setFieldTransform("姓名", "Trim");
    expect(store.sources[0].mappings[0]).toMatchObject({
      target_name: "客户",
      transform: "Trim",
    });
    expect(store.sources[1].mappings[0].target_name).toBe("姓名");
    expect(store.options.output_order).toEqual(["客户", "金额"]);
    store.resetField("姓名");
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
    store.setFieldEnabled("姓名", false);
    expect(store.sources.map((source) => source.mappings[0].enabled)).toEqual([false, true]);
    store.setFieldEnabled("姓名", true);
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
