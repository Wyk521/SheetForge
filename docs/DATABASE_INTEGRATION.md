# SheetForge × pg-table-importer 融合说明

## 设计边界

融合以 SheetForge 为唯一用户工作台，并把两类输出放在同一条数据处理管线之后：

```text
选择数据源
  → 识别/修改表头
  → 交集、并集、修正表头、汇总或关联
  → 清洗、筛选、去重、输出字段排序
  → 处理前检查
  → Excel XLSX 或 PostgreSQL COPY
```

这样可以保证数据库收到的列名和行内容与“合并结果预览”一致。不能让 pg-table-importer 再次扫描原文件，否则 SheetForge 中的逐表表头行、多行表头、字段改名和高级处理规则会被绕过。

## PostgreSQL 能力复用

`src-tauri/Cargo.toml` 将 pg-table-importer 固定到提交 `55c9f7df4897307121ac37f7066cc892d4c27ba4`。`database.rs` 只负责把 SheetForge 的最终行流接到其公开组件：

- `config`：读取和保存无密码 TOML 连接配置；
- `credentials`：Windows Credential Manager 等系统凭据管理器；
- `connection`：SSL 和 PostgreSQL 连接；
- `table`：schema 校验、TEXT NULL 建表、append 前置检查、abort/append/truncate/replace、COPY FREEZE 和表持久性；
- `identifier`：中文、关键字、双引号和超过 63 字节的字段名映射；
- `copy_binary_encoder` / `copy_encoder`：8 MiB 批次的 PostgreSQL 二进制或文本 COPY 编码。

连接配置路径和凭据服务名沿用 pg-table-importer，所以 GUI 与 CLI 可以共用已有连接。密码不会进入 TOML、合并方案、日志或完成事件。

## 交互逻辑

底部始终显示“输出到”切换：

- `Excel 文件` 显示文件路径、预计 Sheet 和“开始合并”；
- `PostgreSQL` 显示当前连接、`schema.table`、表存在策略和“导入数据库”。

“长期连接配置”和“本次导入目标”使用两个独立对话框，避免日常导入时误改连接参数：

1. 顶部“数据库连接”单独管理连接的新建、选择、测试、保存和删除；密码留空时读取系统凭据或 `PGPASSWORD`。应用首次启动且没有任何连接时自动打开这里。
2. 用户切换到 PostgreSQL 时只打开紧凑的“本次导入目标”，从下拉框选择已保存连接，再配置 schema、表名、表存在策略、COPY 格式、空字符串语义、快速提交和 UNLOGGED。
3. `truncate` 和 `replace` 在开始导入前再次确认。
4. 取消导入会关闭行流并丢弃当前数据库事务，不保留部分数据。

默认值偏向安全和可恢复：`abort`、二进制 COPY、LOGGED 表、不开快速提交。UNLOGGED 和快速提交必须由用户显式选择，界面同时说明崩溃恢复和复制风险。

## 数据语义

- 所有目标业务字段均为 `TEXT NULL`，避免前导零、身份证号、超长数字和日期外观字符串被推断为错误类型。
- SheetForge 的真实空单元格写为 SQL NULL；CSV 空字段是否写为 NULL 由“空字符串按 NULL 导入”控制。
- 字符串 `NULL` 和 `\\N` 仍是普通文本。
- 手动表头映射、字段清洗、来源列、筛选、去重、按键汇总和横向关联全部在 COPY 前完成。
- append 会沿用 pg-table-importer 的字段存在性、文本兼容类型和额外 NOT NULL 字段校验。

## 验证

本地静态与回归检查：

```powershell
npm ci
npm run build
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

真实数据库验收建议使用专门的临时 PostgreSQL 实例，至少覆盖：

1. 新建表 + 中文/空格/关键字/超长字段；
2. 修正表头后核对数据库列名和数据；
3. append 字段不匹配时应在 COPY 前失败；
4. truncate/replace 取消后原事务回滚；
5. CSV 的空字符串、引号、换行、反斜线和 `NULL`/`\\N`；
6. 连接中断或主动取消后不留下部分行。
