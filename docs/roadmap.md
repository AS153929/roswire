# RosWire 路线图（Roadmap）

> 最后更新：2026-06-12
> 基准分支：`main`
> 当前版本：`v0.1.3`（MVP / Beta 候选）
> 关联文档：[`feature-checklist.md`](feature-checklist.md)（功能总账）、[`develop-plan.md`](develop-plan.md)（实现规格）、[`mcp-server-development-plan.md`](mcp-server-development-plan.md)（服务化设计）、[`production-readiness.md`](production-readiness.md)（生产门槛）

本文是 `roswire` 的**对外路线图**：用里程碑和时间视角（Now / Next / Later）说明项目要去哪、按什么顺序去、每一步以什么标准算"到达"。它面向使用者、贡献者与协作 Agent，回答"接下来会做什么、什么时候算完成"。

## 这份路线图怎么读

- **它是导航，不是细节**。具体功能条目、设计取舍和验收门槛分别记录在下列文档中，路线图只做编排与链接，不重复内容：

  | 你想知道 | 看这里 |
  | --- | --- |
  | 哪些功能已交付 / 规划中（逐条 + 优先级） | [`feature-checklist.md`](feature-checklist.md) |
  | 核心 CLI 的实现规格与约定 | [`develop-plan.md`](develop-plan.md) |
  | MCP 服务化与 Agent 模式的设计 | [`mcp-server-development-plan.md`](mcp-server-development-plan.md) |
  | 进入生产级稳定版的硬性门槛 | [`production-readiness.md`](production-readiness.md) |
  | 真机 / CHR 兼容性验收 | [`routeros-acceptance-matrix.md`](routeros-acceptance-matrix.md) |
  | 发布与安装流程 | [`release.md`](release.md)、[`installation.md`](installation.md) |

- **以"完成标准"代替"日历日期"**。本路线图刻意不承诺具体交付日期；每个里程碑用可验证的**退出标准（exit criteria）**界定完成与否。顺序是确定的，节奏是里程碑驱动的。
- **状态语义**：✅ 已交付 / 🚧 进行中 / 🔭 规划中 / 🧪 实验性（默认关闭）。

## 北极星（愿景）

让 **AI Agent 与自动化脚本**能以稳定、可解析、可组合、安全可控的方式操作 MikroTik RouterOS —— 先有确定性的 JSON-first CLI，再把同一套执行核心安全地服务化（MCP），最终支持受控的实验性 Agent 自治诊断，而**始终默认只读、写操作显式授权**。

## 当前位置

`roswire` 已完成 MVP 功能闭环，处于 **Beta 候选**：

- ✅ JSON-first CLI 与严格 `stdout`/`stderr` 流隔离、稳定错误码与脱敏 `--debug`
- ✅ 配置 / 多 profile / 密钥后端（keychain / env / encrypted / plain）/ 自描述（`commands`/`schema`/`help`）/ `doctor` 诊断
- ✅ 协议层：原生 API、API-SSL、v7 REST，v6/v7 方言，协议自动探测与优先级；TLS 证书指纹 pinning
- ✅ SSH/SFTP 上传下载与 import / export / backup 文件工作流
- ✅ JSONL 日志、保留策略、远端 schema 缓存 TTL/refresh
- ✅ 发布工程与安装文档、多平台 keychain smoke、生产级门槛定义

> 逐条交付清单见 [`feature-checklist.md` 第一部分](feature-checklist.md)。**尚未**声明生产级稳定版——原因见下方 R1。

## 路线图总览

| 视野 | 里程碑 | 目标版本（示意） | 主题 | 状态 |
| --- | --- | --- | --- | --- |
| **Now** | R1 | `v0.2` | 生产级稳定版（关闭真机矩阵与门槛） | 🚧 |
| **Next** | R2 | `v0.3` | 服务化地基（执行核心抽象 + 配置 include） | 🔭 |
| **Next** | R3 | `v0.4` | MCP Server v1（只读 + 自描述工具） | 🔭 |
| **Later** | R4 | `v0.5` | MCP 写 / 文件工具（默认禁用，显式开启） | 🔭 |
| **Later** | R5 | `v0.x` | 实验性 Agent 模式（默认关闭，feature 门控） | 🧪 |
| 持续 | — | 跨版本 | 命令覆盖、真机矩阵、远端探测、文档 | 🚧 |

---

## 里程碑详情

### R1 · 生产级稳定版 — Now 🚧

把"功能闭环的 Beta"推进到"可面向生产自动化推荐"的 `1.0` 前稳定线。

- **目标**：消除 MVP/Beta 与 Production-stable 之间的歧义边界。
- **关键交付物**：
  - 真实 RouterOS **v6** 与 **v7/CHR** 的验收记录（API / API-SSL / REST、SSH/SFTP/SCP、import/export/backup）。
  - 发布物可复现 + 校验和；多平台 keychain 原生 smoke 记录。
  - 质量门（`cargo fmt --check` / `clippy -D warnings` / 测试 / 依赖审计）与安全门常态化。
- **退出标准**：[`production-readiness.md`](production-readiness.md) 中**全部 P0 blocker 关闭**（核心剩余项为真机矩阵记录，见 [#60](https://github.com/AS153929/roswire/issues/60)、keychain 原生 smoke 见 [#62](https://github.com/AS153929/roswire/issues/62)），Go/No-Go 判定为 Go。
- **关联**：[`routeros-acceptance-matrix.md`](routeros-acceptance-matrix.md)、[`routeros-local-integration.md`](routeros-local-integration.md)、[`release.md`](release.md)。

### R2 · 服务化地基 — Next 🔭

在不改变 CLI 对外契约的前提下，为 MCP/Agent 复用打好内部地基。**对纯 CLI 用户无感知。**

- **目标**：执行逻辑与 `stdout`/`&Cli` 解耦，返回结构化 payload；引入分文件配置。
- **关键交付物**：
  - **执行核心抽象**：executor 可被 CLI 与非 CLI（MCP）共同调用，复用 mapping、raw 安全门、协议层与错误模型；引入 `ExecutionMode`（Readonly / WriteAllowed / ConfigAllowed / TransferAllowed）。
  - **配置 include**：每 profile 一文件（连接 + secret + 该 profile 的策略同文件），相对路径 + 简单 glob，权限校验，稳定展开顺序与冲突报错。
- **退出标准**：CLI 输出 / 错误码 / 脱敏与现状**逐字节一致**；新执行核心被测试覆盖；`config inspect` 能标注每个 profile 的来源文件。
- **关联**：[`mcp-server-development-plan.md`](mcp-server-development-plan.md) Phase 1–2 / 第 6 节；[`feature-checklist.md` 第二部分](feature-checklist.md)（P0/P1）。

### R3 · MCP Server v1（只读） — Next 🔭

把已有能力安全地服务化，面向远程 / 团队 AI client。

- **目标**：`roswire mcp serve`，**仅 Streamable HTTP** 传输，**默认只读**。
- **关键交付物**：
  - Server 骨架与运行时（`tokio` + HTTP + MCP SDK），`spawn_blocking` 有界线程池、并发上限、双层超时、panic 隔离。
  - 认证与暴露面：bearer token；非 loopback 无 token 拒绝启动；TLS 由前置反代终止；审计日志（不落 secret）。
  - 风险分级与策略裁剪：全量 catalog + policy 裁剪（高风险工具可见但 `disabled_by_policy`）；MCP policy 与 roswire mapping **双重校验**。
  - 只读工具：自描述（`server_info`/`list_profiles`/`profile_inspect`/`list_tools`/`doctor`/`schema`/`explain_error` 等）与 RouterOS 只读（interfaces/addresses/routes/firewall/wireguard/system、通用只读 command、`raw .../print`）。
- **退出标准**：默认配置下仅暴露只读能力；server 侧独占凭据授权（client 不能选 host/凭据/raw 写/文件）；只读工具全部经双重校验并保留 `selected_protocol`、错误码与脱敏上下文。
- **关联**：[`mcp-server-development-plan.md`](mcp-server-development-plan.md) Phase 3–5 / 第 5、7、8、9 节。

### R4 · MCP 写 / 文件工具（显式开启） — Later 🔭

在只读 MCP 稳定后，按 profile 显式开启高风险能力，默认全部禁用。

- **目标**：写、文件传输、config-mutation 能力以 **opt-in + allowlist + dry-run-first** 的方式接入 MCP。
- **关键交付物**：`routeros_write_*`、`routeros_raw_write`（单独授权，不继承普通写权限）、`roswire_file_upload`/`download`、`import`/`backup_download`（需 dry-run 与人工确认）。
- **退出标准**：未显式开启时一律 `disabled_by_policy`；任一校验层拒绝都不连接设备；safe-write / dry-run / per-tool allowlist 生效并有审计。
- **关联**：[`mcp-server-development-plan.md`](mcp-server-development-plan.md) Phase 6（P2）。

### R5 · 实验性 Agent 模式 — Later 🧪

**仅在只读 MCP 稳定后启用、不进入默认构建、不在第一版承诺。** 这是探索性方向，包含一个 P2 的确定性层与一个 P3 的实验性 LLM 层；后者默认关闭并 feature 门控。

- **目标**：在严格只读与工具 allowlist 内，提供受控的自动诊断能力。
- **关键交付物**：
  - **确定性诊断（Phase 7，P2）**：不引入 LLM 的 job/resource 管道，runbook 用确定性决策树实现，可被 `cargo test` 断言；大部分价值集中在这一层。
  - **LLM ReAct 层（Phase 8，P3，`--features agent`）**：plan-then-execute 推理，OpenAI 兼容 client（key 走 secret 后端），step/token/时长/花费/并发上限与 cancellation。
- **退出标准**：LLM 层默认 `enabled=false`，纯 CLI 与只读 MCP 用户**零 OpenAI 依赖、零体积负担**；agent 永不暴露写/文件/config-mutation 工具；全部决策与工具调用进审计；数据出域、脱敏与 prompt injection 缓解已评审。
- **关联**：[`mcp-server-development-plan.md`](mcp-server-development-plan.md) 第 14 节；[`feature-checklist.md` 第二部分](feature-checklist.md) Phase 7（P2）/ Phase 8（P3）。

### 跨版本持续工作 🚧

不绑定单一里程碑、贯穿各版本推进：

- **命令覆盖扩展**：优先 `/ip/firewall`、`/ip/route`、`/interface/wireguard` 等高频只读/审计场景。
- **远端 schema 真机探测**：当前 `--remote` 仅返回 `degraded` 静态快照（`REMOTE_PROBE_NOT_IMPLEMENTED`）；真实的设备版本、协议能力、可观测字段与运行时枚举探测为已知缺口，逐步补齐。
- **真机 / CHR 矩阵**：随设备可得性持续补充 v6/v7 记录。
- **文档与 Agent skill**：与能力同步更新，保持 README / checklist / 本路线图一致。

---

## 版本与发布节奏

- 采用 [语义化版本](https://semver.org/lang/zh-CN/)；`1.0` 之前次版本号（`0.x`）可能包含破坏性调整，但 **JSON-first 输出契约的稳定性优先保证**。
- 上表中的目标版本（`v0.2` / `v0.3` …）为**主题示意**，用于表达顺序与范围，不是日期承诺；实际版本号以 release 为准。
- 发布流程、构建矩阵与校验和见 [`release.md`](release.md)。

## 非目标（与 `develop-plan.md` 一致）

- 不替代 WinBox / WebFig / RouterOS 交互式终端。
- 不在 CLI 内保存长期会话或连接池。
- 不默认输出彩色文本、动画、进度条或分页器；不在命令行交互式询问密码或二次确认。
- 不向 MCP client 透传 RouterOS 密码 / SSH key / profile secret。
- 不提供万能 `execute_shell` 或绕过 mapping / raw 安全门 / 脱敏的逃逸路径。

## 路线图治理（防范围漂移）

- **路线图与 [`feature-checklist.md`](feature-checklist.md) 是范围的唯一来源。** 任何开发都应落在二者之中。
- 清单外的新增能力，须先回到本文与开发计划评审、确认所属里程碑与优先级（P0–P3）后再实现，避免"顺手扩张"。
- 红线不可在开发中途擅自突破：**MCP 第一版只读、写操作默认禁用、Agent 模式默认关闭。**
- 里程碑顺序可因真机矩阵、依赖或反馈而调整，但调整需在本文"变更记录"留痕。

## 路线图变更记录

| 日期 | 变更 |
| --- | --- |
| 2026-06-12 | 首次发布路线图：基于 `v0.1.2` 现状，定义 R1–R5 里程碑与 Now/Next/Later 视野。 |
