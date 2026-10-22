# Hermes 目录发现：原生隔离对照验收

日期：2026-10-22。结果：此次 fixture 中，修复后的 Hermes **目录发现**与已安装原生 iterator 的 9 条路径一致；缺失和额外曝光路径均为 0。分类链接与物理来源别名均保持只读。此结果不代表完整原生 catalog 兼容，也不是模型选择、调用或真实用户配置验收。

## 版本与隔离范围

- 原生源码：`/Users/tushaokun/.hermes/hermes-agent`，commit `76d832d3857551a029c4b39c23945eb47c16fe5b`，版本 **0.20.0**（本次重新读取 HEAD、`pyproject.toml` 与 `hermes_cli/__init__.py` 核对）。原生输入复用先前保存的 iterator 与 catalog 回执，不重新运行模型。
- SkillRoster：本工作区 `cargo build` 成功，debug binary **1.8.46**。本验收使用工作区未提交修改构建，不能用当前 Git HEAD 单独代表被测实现。
- Fixture 根：`/var/folders/25/mpff0p7j7slgl5fj961tl7480000gp/T/skillroster-hermes-native-vd2vn5nh`。后文路径均相对此根。
- 仅以 `--home <fixture>/home`、`--source-root <fixture>/approved-category` 扫描；新的状态目录为 `state-after-09mlyq2q`。未修改真实 HOME、Hermes 配置或 Skill 内容，没有运行原生循环链接 fixture。
- 上游当前源码与已安装版本分开记录，详见 [Hermes 机制研究](../research/hermes-skill-discovery.md)。

执行命令的参数结构：

```sh
cargo build
target/debug/skillroster --json \
  --home "$fixture/home" \
  --state-dir "$fixture/state-after-09mlyq2q" \
  --source-root "$fixture/approved-category" scan
```

这里 `$fixture` 仅表示上述临时 fixture 根；没有设置或替换环境变量 `HOME`。

## 输入与结果

原生 iterator 保存于 `native-paths.json`，共 **9 条 lexical entrypoint 路径**；原生 catalog 保存于 `native-catalog.stdout`，共 **8 个名字**。`direct-link` 与 `direct-original` 指向同一 package，原生 catalog 合并这两个路径视图为 `direct-linked`。目录 placement 计数与解析后的 catalog 名称计数必须分开。

| 对照项 | 修复前 `comparison-before.json` | 修复后 `state-after-09mlyq2q/comparison-after.json` |
| --- | --- | --- |
| Hermes `default_exposed=true` 路径 | 6 | 9 |
| 相对原生遗漏 | 3 | 0 |
| 原生之外额外曝光路径 | 0 | 0 |
| Hermes Skills 根 `discovery_complete` | true，尽管遗漏 3 条 | true，fixture 内路径对齐 |
| Snapshot `hermes_discovery_contract_version` | 缺失 | 1 |

三条原先遗漏路径现在均存在且曝光：

| 相对 Hermes Skills 根的入口 | 修复后权限 |
| --- | --- |
| `category-link/child/SKILL.md` | `shared_read_only`，`governable=false` |
| `parent/group/deep/SKILL.md` | `mutable`，`governable=true` |
| `target/legit/SKILL.md` | `mutable`，`governable=true` |

其余原生路径为 `.backup/hidden/SKILL.md`、`direct-link/SKILL.md`、`direct-original/SKILL.md`、`parent/SKILL.md`、`plain/SKILL.md`、`scripts/category/SKILL.md`。这些全部保持曝光。`.archive/archived/SKILL.md` 与 `parent/references/old/SKILL.md` 保留 inventory，但 `default_exposed=false`，没有混入上述 9 条。

分类链接 `home/.hermes/skills/category-link → approved-category` 本身没有 `SKILL.md`。其子入口的 `link_status=not_link`，但祖先链接使它的 physical directory 落在 `approved-category/child`。该 placement 为 `owned_by_agent=true`、`shared_read_only`、`governable=false`。同一物理 package 的 agentless source placement 也为 `shared_read_only`、`governable=false`、`default_exposed=false`；来源扫描没有绕过链接祖先的只读限制。相对地，直接 package 链接 `direct-link → direct-original` 仍按既有合同处理，两条 placement 都为 `mutable`，链接项 `link_status=valid`。

## 旧 Snapshot 拒绝复用

通过 SQLite backup API 将修复前 fixture 的状态复制到新的 `state-legacy-check-xsjov3gn`，在该副本上执行当前 binary 的 `report`。回执为 `ok=false`：

```json
{
  "code": "content_identity_rescan_required",
  "reason": "legacy_hermes_discovery_requires_rescan",
  "next_action": "scan",
  "files_changed": false,
  "state_files_changed": false
}
```

完整回执位于该副本的 `report.json`；原始修复前状态保持独立。此项直接验证旧发现合同不能被 Report 继续采用；其他调用入口由下述回归测试核验。

## 可追溯回执与范围限制

新状态目录保留 `scan.json`、`payload.json`、`comparison-after.json`，对应 Snapshot `scan_0000000000002bef18e0e4c84e804a40`。原生回执 `native-catalog.stderr` 为空。输入 SHA-256 为：

| 文件 | SHA-256 |
| --- | --- |
| `native-paths.json` | `73b70777680c46a77ee7102029f2036dbfdc81884d5b27f4741d2a5f95c36020` |
| `native-catalog.stdout` | `be3fc0dc8e36f38cffb1d13ff2d186aa58b4ed35823ac8cf6dbf719d8cc0fc6a` |
| `comparison-before.json` | `da1a9da93b0ffb750c835587f61df6ba4997201ce2afb376a12eb030381b8958` |

本轮验证 Hermes 指定目录内的深层入口、`target`、普通隐藏目录、support 排除、直接 package 链接、已批准分类链接及其物理别名权限；不实现或验证 Hermes `HERMES_HOME`/profile、`external_dirs`、`create_dir`、project trust、`skills.disabled`、platform/environment/apps、同名优先级与 plugin/session catalog 语义。用户配置或运行时过滤可以令真实 catalog 少于目录投影；`discovery_complete=true` 只说明本轮受界限约束的目录发现没有报告截断。

## 回归验证

- `cargo build` 与 `cargo run -- --help` 成功。
- `bash scripts/ci-full-check.sh` 返回 0：格式、全部 target/feature 的严格 Clippy、575 项 Rust 测试、152 项 Node 测试及 README/安装/发布归档校验通过；保留 1 项原有 ignored 测试。
- 新增 8 项 Scan 单元回归覆盖递归与 support 规则、独立链接别名与循环、深度边界、持久只读权限、扫描期间重定向、入口文件链接逃逸、链接根的共享事实及普通文件链接。
- 新增 8 项公开 CLI 回归覆盖原生路径集合、未批准来源拒读与显式批准恢复、只读别名的 Plan 拒绝、扫描后链接重定向的精确加载拒绝、旧 Snapshot 的 Report/Find/Setup/check/Apply 重扫要求，以及下面两项审查发现的缺陷。
- App/error 单元回归覆盖缺失与错误的合同版本、非 Hermes 根不受影响、单一 Scan 恢复动作。旧 Ready Plan 仍可查看，Apply 拒绝后保留 Ready；已执行的历史 Plan 也仍可查看。

验证日志在本机 `/tmp/skillroster-hermes-ci-20261022.log` 与 `/tmp/skillroster-hermes-help-20261022.log`。本轮在 macOS 本地完成，没有替代 Linux/Windows 发布构建或真实 Agent 会话验收。

## 独立审查后的修正

1. `.hermes/skills` 本身链接到共享目录时，初版只标记 placement 为只读，未标记根，Bootstrap Setup 因而仍可写入。修复使根与 placement 权限一致；同时去掉 Bootstrap 绕过通用 mutation-scope 校验的分支，在准备与 Apply 时检查操作和只读包的逻辑/物理路径重叠。公开测试覆盖已有自定义 Bootstrap、空共享根，以及模拟已有 Ready Plan 时的 Apply 拒绝；原文件和 Ready 状态均保留。
2. `guide.md -> README.md` 这类普通文件链接被误当目录而造成扫描不完整。已批准且确认目标为常规文件的链接现在跳过目录发现；未批准或不可确认的目标仍保持保守边界。公开测试确认健康根的完整发现及后续普通 Roster 规划。

修复后独立审查在临时 HOME 中重放了已有 Bootstrap、空根、root-level `SKILL.md`、Ready Plan 和普通文件链接场景，未发现剩余阻断问题。最终构建再次在原生 fixture 的新状态目录 `state-final-ikkyui0j` 扫描：9 条路径完全一致，两个分类链接/来源 placement 仍为共享只读；该目录保存最终 `scan.json`、`payload.json` 和 `comparison-after.json`。
