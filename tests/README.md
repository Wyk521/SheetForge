# 测试运行说明

## 常规检查

Node.js 22、稳定版 Rust；在项目根目录运行：

```sh
npm ci
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo audit --file src-tauri/Cargo.lock
```

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

用例要求 PostgreSQL 17，环境缺失、连接失败或版本不符均失败。每个用例创建独立的 `sheetforge_dt_<进程>_<序号>` schema，成功后删除；应使用可销毁的专用数据库。用例涵盖二进制/文本 COPY、NULL 与转义、清洗/筛选/去重/列顺序/来源列、XLSX 逐格对照、追加校验、失败和取消后的事务回滚、取消后重试。

普通 `cargo test` 将数据库和大文件性能用例列为 ignored；Linux CI 的独立作业显式执行数据库用例，不以环境缺失跳过。Windows CI 执行普通 Rust 测试，其中原生更新测试复制测试程序到临时 Unicode 路径，实际替换和启动，再验证无效 EXE/启动后退出时的恢复。

## 性能复测

macOS/Linux 的 Python 3 和 `/usr/bin/time`；同一机器、同一 Rust 工具链和构建模式，基线与重构后各运行三次，期间避免运行其他重负载任务：

```sh
# 在基线版本生成固定样本：100 万行、8 列 CSV；10 Sheet、每 Sheet 1 万行的 XLSX。
python3 scripts/benchmark.py /tmp/sheetforge-bench baseline --generate
# 切换到待验收版本后，复用同一份样本。
python3 scripts/benchmark.py /tmp/sheetforge-bench final --compare baseline
```

脚本按进程测量完整扫描和合并，分别报告中位耗时与最大驻留内存中位数，并比较输出工作表、共享字符串和样式 XML。任一指标增加超过 10% 或输出内容变化都会失败。生成文件和日志留在指定临时目录，不提交到仓库。

本轮同机 debug 模式验收（基线为首个测试保护提交，生产引擎尚未拆分）：

| 样本 | 基线耗时 | 重构后耗时 | 基线 RSS | 重构后 RSS | 增幅（耗时 / RSS） |
|---|---:|---:|---:|---:|---:|
| 百万行 CSV | 57.58 s | 57.81 s | 14,696,448 B | 15,417,344 B | +0.4% / +4.9% |
| 多 Sheet XLSX | 12.91 s | 12.96 s | 15,499,264 B | 16,400,384 B | +0.4% / +5.8% |

两种输出的内容校验一致。以上是引擎进程数据，不代表完整桌面窗口的内存占用。真实 PostgreSQL 17 的五组用例已在本地临时容器通过；Windows 结果以 PR 和发布 Actions 的实际结论为准。桌面交互验收与自动化结果分开记录，不能用 macOS 或编译通过代替 Windows 原生测试。

## 固定格式样本

`fixtures/calamine/date.*.hex` 是 [Calamine v0.36.1 的测试文件](https://github.com/tafia/calamine/tree/v0.36.1/tests)的十六进制编码副本。测试还原后通过真实 XLS、XLSB、ODS 读取和输出管线，保留上游 [MIT 许可证](fixtures/calamine/LICENSE-MIT)。

| 文件 | 原始文件 SHA-256 |
|---|---|
| date.xls | e3eb3391a371cc1e0b85d94b9f1dd94bbcedf19733ed0ca449e2d269af758e9e |
| date.xlsb | 68d36adf4d4de3890209e786eeb89a98778d25158628f83e96587e652fbb4eca |
| date.ods | d65f3198bc73f2fad356e1b097215fc2f9a833fb802638217e5adc27fbba0466 |
