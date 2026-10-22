# 测试运行说明

## 常规检查

Node.js 22、稳定版 Rust；在项目根目录运行：

```sh
npm ci
npm run validate -- --postgres --output /tmp/sheetforge-validation
# 追加新旧版本性能对照及千万行压力验证：
npm run validate -- --postgres --performance --large --output /tmp/sheetforge-full-validation
```

一个入口依次执行格式、Clippy、Rust、审计、前端测试与构建；失败会打印日志尾部，完整日志与 JSON 汇总保留在输出目录。PostgreSQL 使用自动创建并清理的专用临时容器。性能矩阵、输入规模和浏览器批量页面见 [批量验证说明](../scripts/performance/README.md)。

`npm test` 先类型检查源码与测试，再执行 `node:test` 界面结构检查和 Vitest 状态/异步行为测试。测试不要求打开桌面窗口。真实文件用例扫描、合并并读回 XLSX，逐格验证输出值及类型；预览按来源抽样，不能把样本行数等同于最终合并行数。

## 真实 PostgreSQL 17

使用隔离的 PostgreSQL 17 实例，设置以下环境变量后明确运行集成用例：

```sh
export SHEETFORGE_TEST_POSTGRES_PORT=5432
export SHEETFORGE_TEST_POSTGRES_DATABASE=sheetforge_dt
export SHEETFORGE_TEST_POSTGRES_USER=postgres
export SHEETFORGE_TEST_POSTGRES_PASSWORD='专用测试实例密码'
cargo test --manifest-path src-tauri/Cargo.toml --locked postgres_integration -- --ignored
```

用例要求 PostgreSQL 17，环境缺失、连接失败或版本不符均失败。每个用例创建独立的 `sheetforge_dt_<进程>_<序号>` schema，成功后删除；应使用可销毁的专用数据库。用例涵盖二进制/文本 COPY、缺失值与空文本的 NULL 区别、转义、真实 XLSX 数字/日期/时间/布尔值与两种合并模式的输出一致性、清洗/筛选/去重/列顺序/来源列、XLSX 逐格对照、追加校验、失败和取消后的事务回滚、取消后重试。

普通 `cargo test` 将数据库和大文件性能用例列为 ignored；Linux CI 的独立作业显式执行数据库用例，不以环境缺失跳过。Windows CI 执行普通 Rust 测试，其中原生更新测试复制测试程序到临时 Unicode 路径，实际替换和启动，再验证无效 EXE/启动后退出时的恢复。

## 性能复测

macOS/Linux 的 Python 3 和 `/usr/bin/time`；同一机器、同一 Rust 工具链和构建模式，基线与重构后各运行三次，期间避免运行其他重负载任务：

```sh
# 使用当前脚本构建旧检出，避免旧基线没有 benchmark.py 的问题。
# fb1a2d6 包含性能测试入口，生产引擎尚未拆分。
git worktree add --detach /tmp/sheetforge-baseline fb1a2d6eea403e82990a8a1c9aeaa77e77c08d60
# 共用构建缓存；脚本仍按每个检出的锁文件重建测试程序。
export CARGO_TARGET_DIR="$PWD/src-tauri/target"
python3 scripts/benchmark.py /tmp/sheetforge-bench baseline --repo /tmp/sheetforge-baseline --generate
python3 scripts/benchmark.py /tmp/sheetforge-bench final --repo "$PWD" --compare baseline
# 仅清理上面新建的临时检出；允许删除测试自动生成的 schema。
git worktree remove --force /tmp/sheetforge-baseline
```

脚本在 JSON 中记录源码提交、是否有未提交改动、Rust/Cargo 版本、系统、构建模式及输入文件 SHA-256；新报告之间比较时要求工具链、系统、构建模式和样本一致。脚本按进程测量完整扫描和合并，分别报告中位耗时与最大驻留内存中位数，并比较输出工作表、共享字符串和样式 XML。任一指标增加超过 10% 或输出内容变化都会失败。生成文件和日志留在指定临时目录，不提交到仓库。

v0.7.10 同机 debug 模式验收（基线为首个测试保护提交，生产引擎尚未拆分）：

| 样本 | 基线耗时 | 重构后耗时 | 基线 RSS | 重构后 RSS | 增幅（耗时 / RSS） |
|---|---:|---:|---:|---:|---:|
| 百万行 CSV | 57.58 s | 57.81 s | 14,696,448 B | 15,417,344 B | +0.4% / +4.9% |
| 多 Sheet XLSX | 12.91 s | 12.96 s | 15,499,264 B | 16,400,384 B | +0.4% / +5.8% |

两种输出的内容校验一致。以上是引擎进程数据，不代表完整桌面窗口的内存占用。真实 PostgreSQL 17 的六组用例（新增工作簿与两种模式对照）已在本地临时容器通过；Windows 结果以 PR 和发布 Actions 的实际结论为准。桌面交互验收与自动化结果分开记录：macOS 已执行文件选择、来源预览、字段重命名、结果预览和真实 XLSX 写出；Windows 原生自动化已覆盖替换、特殊字符路径与失败恢复。完整 Windows 桌面交互和已安装旧版通过真实更新服务升级到新版尚未验收，不计为通过；当前开发机为 macOS，Windows CI 的原生更新用例使用测试程序及新程序桩，不能替代这两项证据。

## 固定格式样本

`fixtures/calamine/date.*.hex` 是 [Calamine v0.36.1 的测试文件](https://github.com/tafia/calamine/tree/v0.36.1/tests)的十六进制编码副本。测试还原后通过真实 XLS、XLSB、ODS 读取和输出管线，保留上游 [MIT 许可证](fixtures/calamine/LICENSE-MIT)。

| 文件 | 原始文件 SHA-256 |
|---|---|
| date.xls | e3eb3391a371cc1e0b85d94b9f1dd94bbcedf19733ed0ca449e2d269af758e9e |
| date.xlsb | 68d36adf4d4de3890209e786eeb89a98778d25158628f83e96587e652fbb4eca |
| date.ods | d65f3198bc73f2fad356e1b097215fc2f9a833fb802638217e5adc27fbba0466 |
