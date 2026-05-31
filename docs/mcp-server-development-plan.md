# RosWire MCP Server 开发计划

> 日期：2026-06-01
> 范围：本计划只覆盖直连 server 模式与 MCP 工具暴露。后续接入层、离线任务和注册发现能力不在本阶段范围内。
>
> 传输固定：MCP 对外只提供 **Streamable HTTP** 传输（现行 MCP 规范，旧的 HTTP+SSE 已废弃）。本阶段不实现 stdio 传输。
>
> 部署模型：MCP server 面向**远程 / 团队 AI client**。本机使用场景一律走 `roswire` CLI（由 skill 封装），不经过 MCP。也就是说 server 必然监听网络、必然要求认证，loopback 仅用于本机联调。

## 1. 背景与目标

`roswire` 当前是 JSON-first RouterOS CLI。它已经具备 profile、secret、协议选择、命令映射、raw 安全门、文件工作流、doctor、自描述 schema 和结构化错误输出。

下一阶段目标是把这些能力服务化：

- 新增 `roswire mcp serve`，让本机或可信远端客户端通过 MCP 调用 `roswire`。
- 将现有 CLI 能力纳入 MCP tool catalog，而不是另起一套设备操作 API。
- 保持 `roswire` 现有安全边界：profile/secret 留在 server 侧，MCP 客户端不能直接传 RouterOS 凭据。
- 让 AI client 能通过 MCP tool metadata 理解可用能力、参数、风险等级和策略状态。
- 默认只开放只读能力；写操作、文件导入和高风险 raw 操作必须经过本地 policy 显式启用。

## 2. 非目标

- 不实现边缘节点注册、离线队列或异步任务投递。
- 不把 RouterOS 密码、SSH key、profile secret 透传给 MCP 客户端。
- 不提供一个万能 `execute_shell` 或本地进程执行工具。
- 不让模型自由拼接 RouterOS 原生命令后绕过 `mapping`、`validate_raw_safety`、secret 脱敏和错误结构。
- 不在第一版实现长期连接池；每次 tool call 仍按现有 roswire 语义独立执行。

## 3. 设计原则

### 3.1 先抽执行核心，再接 MCP

当前 `src/lib.rs` 的主路径会直接 `println!`。MCP server 需要复用同一套执行逻辑，但返回结构化 payload。

目标结构：

```text
CLI args / MCP tool input
  -> roswire executor
    -> config / introspect / workflow / transfer / RouterOS command
      -> structured payload or RosWireError
  -> CLI prints or MCP returns
```

要求：

- CLI 行为保持兼容。
- MCP tool handler 不重新实现 RouterOS 协议调用。
- 所有 RouterOS 操作继续经过 `mapping::build_protocol_request()` 和 raw safety gate。
- 错误输出继续复用 `RosWireError`、`ErrorContext` 和脱敏规则。

### 3.2 全量 catalog，策略裁剪

“暴露已有功能”分两层：

- catalog 层：现有能力都应在 MCP metadata 中可被发现。
- policy 层：是否可调用由本机配置决定。

也就是说，高风险 tool 可以出现在工具清单里，但返回 `disabled_by_policy`，并在描述中说明需要 server 侧管理员显式启用。

### 3.3 server 侧拥有授权解释权

MCP 客户端可以选择工具和传入参数，但不能决定：

- 使用哪个 RouterOS host。
- 使用哪个 RouterOS user/password。
- 是否允许 raw 写操作。
- 是否允许文件导入、脚本执行或会影响连通性的写操作。

这些都由 server 侧 profile 与 policy 决定。

## 4. 配置模型

新增 `[mcp]` 配置段，放在 `~/.roswire/config.toml`：

```toml
[mcp]
enabled = true
bind = "127.0.0.1:8765"
auth = "bearer"
token_env = "ROSWIRE_MCP_TOKEN"
default_mode = "readonly"
max_request_body_bytes = 1048576
max_result_bytes = 4194304
tool_timeout_seconds = 30

[mcp.profiles.office_wifi]
enabled = true
mode = "readonly"
allow_raw_print = true
allow_raw_write = false
allow_file_transfer = false
allow_config_mutation = false

[mcp.profiles.office_wifi.tools]
doctor = true
config_inspect = true
schema = true
routeros_print = true
workflow_script_put = false
file_transfer = false
routeros_write = false
```

默认值：

- 只监听 `127.0.0.1`。
- 非 loopback bind 必须显式配置认证。
- 默认 `readonly`。
- 默认禁用 raw 写操作、文件上传/导入、RouterOS 写操作和 config mutation。

### 4.1 Profile include

profile 数量多时，单个 `config.toml` 会变成维护瓶颈。MCP server 计划应同时补上核心配置 include 能力，并采用“每个 profile 一个文件”的组织方式：设备连接配置、secret 引用和该 profile 的 MCP policy 放在同一个文件里，避免连接配置和访问策略分裂。

推荐主配置：

```toml
version = 1
default_profile = "office_wifi"
include = [
  "profiles/*.toml",
]

[mcp]
enabled = true
bind = "127.0.0.1:8765"
auth = "bearer"
token_env = "ROSWIRE_MCP_TOKEN"
default_mode = "readonly"
```

推荐 profile 文件：

```toml
# ~/.roswire/profiles/office_wifi.toml
[profiles.office_wifi]
host = "192.168.88.1"
user = "admin"
protocol = "auto"
routeros_version = "auto"
transfer = "ssh"

[profiles.office_wifi.secrets.password]
type = "keychain"
service = "roswire"
account = "profiles/office_wifi/password"

[mcp.profiles.office_wifi]
enabled = true
mode = "readonly"
allow_raw_print = true
allow_raw_write = false
allow_file_transfer = false
allow_config_mutation = false

[mcp.profiles.office_wifi.tools]
doctor = true
config_inspect = true
schema = true
routeros_print = true
workflow_script_put = false
file_transfer = false
routeros_write = false
```

include 规则：

- include path 相对 `config.toml` 所在目录解析。
- 第一版只允许相对路径和简单 glob，不允许绝对路径、`..`、软链接逃逸 `ROSWIRE_HOME`。
- include 文件必须和主配置一样执行权限检查；Unix/macOS 目标为 `0600`，包含目录目标为 `0700`。
- include 文件只允许补充同名 `[profiles.<name>]`、`[mcp.profiles.<name>]` 和相关 profile 级子表；`version`、`default_profile`、全局 `[mcp]`、`[logging]` 仍只能写在主配置。
- 一个 include 文件最多定义一个设备 profile；如果同时定义 MCP profile policy，policy 名称必须与设备 profile 名称一致。
- 允许纯设备 profile 文件不配置 `[mcp.profiles.<name>]`，此时该 profile 默认不对 MCP 可见，除非全局 policy 后续显式开启。
- include 展开顺序必须稳定，glob 结果按路径字典序排序。
- 同名 profile 或同名 MCP profile policy 在多个文件中重复定义时报错，不做静默覆盖。
- include 文件内第一版不再允许继续 include，避免递归、循环和难以解释的合并顺序。
- `config inspect` 和 `mcp server_info` 应输出每个 profile 的来源文件，但只显示脱敏后的相对路径。
- 错误码应能区分 include 文件不存在、glob 无匹配、权限过宽、重复 profile、非法路径、非法全局字段、单文件多 profile、profile 与 MCP policy 名称不一致。

## 5. MCP Tool 设计

### 5.1 基础工具

这些工具优先实现，用于让 client 了解 server 与可用能力：

| Tool | 说明 | 默认 |
| --- | --- | --- |
| `roswire_server_info` | 返回版本、server 配置摘要、支持的 MCP schema version | enabled |
| `roswire_list_profiles` | 返回可见 profile，不返回 secret | enabled |
| `roswire_profile_inspect` | 返回指定 profile 的脱敏连接配置和 secret 状态 | enabled |
| `roswire_list_tools` | 返回当前 policy 裁剪后的 tool catalog | enabled |
| `roswire_doctor` | 运行本地 doctor，可选 include remote | enabled |

### 5.2 自描述工具

| Tool | 说明 | 默认 |
| --- | --- | --- |
| `roswire_commands` | 返回 roswire 支持的命令目录 | enabled |
| `roswire_help` | 返回单个命令 help | enabled |
| `roswire_schema` | 返回单个命令 schema | enabled |
| `roswire_explain_error` | 返回错误码解释 | enabled |

### 5.3 RouterOS 只读工具

第一版应提供两种形式：

高层工具：

- `routeros_observe_interfaces`
- `routeros_observe_addresses`
- `routeros_observe_routes`
- `routeros_observe_firewall`
- `routeros_observe_wireguard`
- `routeros_observe_system`

通用只读工具：

```json
{
  "profile": "office_wifi",
  "path": ["ip", "route"],
  "action": "print",
  "args": {},
  "flags": ["detail"]
}
```

限制：

- `action` 第一版只允许 `print`。
- `flags` 只允许现有只读 print option。
- `file`、`interval`、`follow`、`follow-only` 等仍按现有规则拒绝。
- 返回结果必须保留 `selected_protocol`、错误码和脱敏上下文。

### 5.4 Raw 只读工具

提供 `routeros_raw_print`，但必须有 policy：

```json
{
  "profile": "office_wifi",
  "path": "/system/resource/print",
  "args": {},
  "flags": ["detail"]
}
```

限制：

- path 必须以 `/` 开头，且最后一段必须是 `print`。
- path 必须通过现有 `normalize_raw_routeros_path()`。
- 如果 profile policy 配置了 allowlist，则 path 必须命中 allowlist。
- 不提供 MCP 层面的 `allow_write` 参数。

### 5.5 写操作与文件工具

写操作和文件工具不在第一阶段默认启用，但 catalog 中需要有稳定位置：

| Tool group | 第一阶段状态 | 启用条件 |
| --- | --- | --- |
| `routeros_write_*` | disabled_by_policy | profile 显式开启，并按 tool allowlist 开放 |
| `routeros_raw_write` | disabled_by_policy | 单独显式开启，不继承普通写权限 |
| `roswire_file_upload` | disabled_by_policy | 显式开启 transfer policy |
| `roswire_file_download` | disabled_by_policy | 可先开放只读下载，但必须限制路径和大小 |
| `roswire_import` | disabled_by_policy | 后续阶段，必须有 dry-run 与人工确认路径 |
| `roswire_backup_download` | disabled_by_policy | 后续阶段，先验证真实设备矩阵 |

## 6. 执行核心改造

### 6.1 新增 executor 模块

建议新增：

```text
src/executor/mod.rs
src/executor/input.rs
src/executor/output.rs
src/executor/policy.rs
```

职责：

- 把 CLI token 或 MCP input 转成统一 `ExecutionRequest`。
- 调用现有 config/introspect/workflow/transfer/RouterOS command handler。
- 返回 `ExecutionOutput`，不直接写 stdout/stderr。
- 提供 `ExecutionMode`：`Readonly`、`WriteAllowed`、`ConfigAllowed`、`TransferAllowed`。

### 6.2 保持 CLI 兼容

现有 `run_with_cli()` 改成薄包装：

```text
parse Cli
  -> executor.execute_cli(cli)
  -> print stdout/stderr exactly as before
```

验收条件：

- 现有 CLI smoke tests 不需要大改。
- JSON payload schema 不因为 MCP 改造发生非必要变化。
- 错误码、hint、context 脱敏保持一致。

## 7. MCP Server 模块

建议新增：

```text
src/mcp/mod.rs
src/mcp/server.rs
src/mcp/tools.rs
src/mcp/schema.rs
src/mcp/auth.rs
src/mcp/policy.rs
src/mcp/runtime.rs
```

CLI 入口：

```bash
roswire mcp serve --json
roswire mcp serve --bind 0.0.0.0:8765 --json
```

server 行为：

- 启动时加载配置、校验 bind/auth/policy；非 loopback bind 缺少 token 时拒绝启动。
- 对外提供 Streamable HTTP 端点与 MCP tool listing。
- 每次 tool call 先认证，再校验 profile，再校验 tool policy，再调用 executor。
- 所有 tool result 都返回结构化 JSON。
- 工具执行错误返回 MCP error，同时携带 roswire structured error payload。

### 7.1 运行时与并发模型

executor 与底层协议栈（`ureq`、`ssh2`）是**同步阻塞**的，而 Streamable HTTP server 需要一个异步运行时。两者通过明确的边界隔离，不把阻塞调用混进 async 任务里：

- 传输层用 `tokio` + 一个 HTTP 框架承载 Streamable HTTP；MCP 协议层优先评估官方 Rust SDK（`rmcp`，支持 streamable-http），避免自己手写 JSON-RPC 帧。具体 crate 与版本在 Phase 0 冻结并 pin。
- 每个 tool call 的实际执行（executor → RouterOS）通过 `spawn_blocking` 投递到**有界**阻塞线程池，绝不在 async 任务里直接做阻塞 I/O。
- 并发上限：限制最大在途请求数与阻塞线程数；超过上限的请求返回结构化 `server_busy` 错误，而不是无限堆积。
- 连接模型不变：每次 tool call 独立按现有 roswire 语义建连，第一版不做连接池。

### 7.2 超时语义（必须显式定义）

`tool_timeout_seconds` 在同步阻塞栈下**无法真正中断**一个卡住的 `ureq`/`ssh2` 调用。第一版采用双层超时，并在文档中讲清边界：

- 底层超时：给协议层配置 connect/read 超时（socket 级），作为硬下限。
- MCP 层超时：用 `tokio::time::timeout` 包住 `spawn_blocking` 句柄；超时后**立即向 client 返回结构化 `tool_timeout` 错误**，但被孤立的阻塞线程会继续跑到底层超时才退出。
- 因此 MCP 层超时必须 ≥ 底层超时，且阻塞线程池要有上限，避免被孤立线程拖垮。

### 7.3 panic 隔离

当前 release profile 设了 `panic = "abort"`，这意味着任一 tool 调用 panic 会**杀掉整个 server 进程**，而非隔离单个请求。Phase 0 必须二选一并写明：

- 方案 A（推荐）：为 server 路径采用 `panic = "unwind"`（独立 profile / feature / 独立 binary），让 tokio 把单个任务 panic 收敛成该请求的 5xx，进程存活。
- 方案 B：保留 `panic = "abort"`，依赖外部 supervisor（systemd 等）重启；接受 panic 时全部在途请求丢失。

无论哪种，都需在 `spawn_blocking` 边界做 `catch_unwind` 兜底并返回结构化 internal error。

## 8. 权限策略

### 8.1 风险等级

每个 tool 标记风险：

| Risk | 含义 | 默认 |
| --- | --- | --- |
| `readonly` | 不修改 RouterOS 状态 | enabled |
| `local_config_read` | 读取本地脱敏配置 | enabled |
| `local_config_write` | 修改本地 roswire 配置 | disabled |
| `routeros_write` | 修改 RouterOS 状态 | disabled |
| `connectivity_risk` | 可能影响连接、路由、防火墙或管理入口 | disabled |
| `secret_sensitive` | 涉及 secret 写入或 secret 后端 | disabled |
| `file_transfer` | 上传、下载或导入文件 | disabled |

### 8.2 双重校验

每次执行都必须同时通过：

- MCP tool policy。
- 现有 roswire command validation。

任何一层拒绝都不应继续连接 RouterOS。

## 9. 传输、认证与暴露面

### 9.1 传输

- 唯一对外传输为 **Streamable HTTP**（MCP 现行规范）；不实现 stdio。
- 端点、会话语义、SSE 流式回包按所选 MCP SDK（Phase 0 冻结）的实现走，不自定义私有协议。
- 因为目标是远程 client，server 默认就是网络可达的；`bind` 默认仍写 `127.0.0.1` 以防误配，真正远程部署需显式改 bind 并满足下方认证要求。

### 9.2 认证

- 非 loopback bind **必须**启用 bearer token，否则拒绝启动（无 dev 例外）。
- loopback bind 允许无 token，仅用于本机联调，必须在启动日志和 `server_info` 中标记 `auth_mode=loopback_dev`。
- token 从环境变量或 secret 后端读取，不写入 config 明文。
- 每个请求在进入 profile/policy 校验前先过认证；认证失败返回 MCP error，不泄露 profile 是否存在。

### 9.3 传输安全（TLS）

- 因为面向远程，bearer token 必须在 TLS 之上传输，否则 token 明文过网。第一版要求：非 loopback 部署时由前置反向代理（nginx/caddy 等）终止 TLS，server 只监听其后的回环/内网地址。
- 文档必须明确：直接把明文 HTTP + bearer 暴露到不可信网络属于误配，`server_info` 应能反映 `tls=external|none` 以便审计。
- 内置 TLS 作为后续增强。

### 9.4 暴露面与审计

- result size limit 在 server 侧强制：执行结果先按 `max_result_bytes` 截断/拒绝再返回，避免大 print 拖垮 client；超限返回结构化 `result_too_large`。
- 复用现有 `logging` 模块记录每次 MCP 调用的审计事件（profile、tool、risk、policy 判定、耗时、结果大小），secret/password/private key 路径不得进入日志。

### 9.5 后续增强

- 内置 TLS。
- 多 token / 多 client policy。
- token scope：按 profile 和 tool group 限制。

## 10. 测试计划

### 10.1 单元测试

- executor 输入输出不写 stdout/stderr。
- readonly policy 拒绝 `add/set/remove`。
- raw print 通过；raw 非 print 拒绝。
- disabled tool 出现在 catalog，但调用返回 policy error。
- profile 不存在、profile disabled、tool timeout、result too large 都有结构化错误。

### 10.2 CLI 回归

继续运行：

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

新增：

```bash
cargo test mcp --all-targets
```

### 10.3 集成测试

- 启动本地 MCP server。
- 查询 tools。
- 调用 `roswire_server_info`。
- 调用只读 `routeros_observe_system`，使用 fake/fixture backend。
- 验证被禁用 tool 的错误。
- 验证 token 缺失、错误 token、loopback dev mode。

### 10.4 RouterOS 验收

沿用现有 RouterOS/CHR 验收矩阵，先覆盖：

- `doctor --include-remote`
- `system resource print`
- `interface print`
- `ip address print`
- `ip route print`
- raw `/system/resource/print`

## 11. 阶段计划

> 优先级说明：**P0 = 必做地基** / **P1 = 第一版目标** / **P2 = 后续增强** / **P3 = 实验性、默认关闭**。优先级表示重要性，不等于实现顺序（Phase 编号才是建议顺序）。

| Phase | 内容 | 优先级 |
| --- | --- | --- |
| 0 | 规格冻结（含传输/运行时/panic/超时冻结） | P0 |
| 1 | Config include（独立，可与 Phase 2 并行或延后） | P1 |
| 2 | Executor 抽象（解除 `&Cli` 耦合） | P0 |
| 3 | MCP Server 骨架（Streamable HTTP + 运行时） | P0 |
| 4 | 只读 RouterOS Tools | P1 |
| 5 | 自描述与诊断工具 | P1 |
| 6 | 受限写操作预研 | P2 |
| 7 | 确定性诊断 runbook + job/resource 管道（无 LLM） | P2 |
| 8 | 实验性 LLM Agent / ReAct 层（引入 OpenAI 配置） | P3（默认关闭） |

### Phase 0：规格冻结

产出：

- 本文档合并。
- MCP tool 命名、risk taxonomy、policy 字段冻结。
- 明确第一版只读默认边界。
- **传输与运行时冻结**：确认传输为 Streamable HTTP；选定并 pin MCP SDK（候选 `rmcp`）、HTTP/async 栈（`tokio` + HTTP 框架）；决定 panic 隔离方案（7.3 的 A 或 B）；定义超时双层语义（7.2）。

完成标准：

- README 或开发文档中能解释 `roswire mcp serve` 的边界与传输（仅 Streamable HTTP）。
- 没有承诺默认开放写操作。
- Cargo 依赖与 panic 策略变更已评估对 CLI 体积/行为的影响。

### Phase 1：Config include（独立先行）

> 与 MCP 正交，建在已较大的 `config/mod.rs` 上风险独立，单独成阶段、单独验证，不作为后续阶段的硬阻塞。

产出：

- config include loader，支持 profile/policy 分文件加载（每 profile 一文件）。
- include 路径、glob、`0600/0700` 权限、来源追踪与第 4.1 节列出的全部错误码。

完成标准：

- `config profiles`、`config inspect` 能看到 include 文件中的 profile，并显示脱敏来源路径。
- include 文件权限、非法路径、glob 无匹配、重复 profile、单文件多 profile、policy 名称不一致都有结构化错误测试。

### Phase 2：Executor 抽象

产出：

- `src/executor/`。
- CLI 改为调用 executor（薄包装），executor 返回结构化 payload 而非自行打印。
- 解除执行路径对 `&Cli` 的直接依赖，改吃中立 `ExecutionRequest`/上下文。
- 现有测试通过。

完成标准：

- CLI 输出与改造前一致。
- executor 可被非 CLI 调用。

### Phase 3：MCP Server 骨架（Streamable HTTP）

产出：

- `roswire mcp serve`，Streamable HTTP 端点。
- `tokio` + HTTP 栈 + 选定 MCP SDK 接入；`spawn_blocking` 边界、并发上限、超时与 panic 兜底落地。
- `server_info`、`list_profiles`、`list_tools`。
- bearer token 验证。

完成标准：

- 本地 server 可启动、查询、关闭。
- 非 loopback 无 token 配置时拒绝启动。
- 单请求 panic 不杀进程（按 7.3 选定方案验证）；tool timeout 能返回结构化错误且不无限堆积阻塞线程。

### Phase 4：只读 RouterOS Tools

产出：

- 高层 observe tools。
- 通用 readonly command tool。
- raw print tool。

完成标准：

- 所有只读 tool 经过 policy 和 mapping 双重校验。
- 禁用写操作的测试覆盖到位。

### Phase 5：自描述与诊断工具

产出：

- commands/help/schema/explain-error 工具。
- doctor 工具。
- profile inspect 工具。

完成标准：

- AI client 可仅凭 MCP metadata 理解可用工具和参数。
- secret、password、private key 路径不出现在 tool result 中。

### Phase 6：受限写操作预研

产出：

- safe-write policy 设计。
- dry-run first 机制。
- per-tool allowlist。

完成标准：

- 不默认启用。
- 每个写 tool 都有独立风险说明、测试和回滚/验证策略。

### Phase 7：确定性诊断 runbook + job/resource 管道（P2）

> 这是 agent 设想里风险最低、和 roswire 身份最契合的一半：固定排障剧本 + 异步 job + MCP resource，**不引入任何 LLM**。

产出：

- 预设诊断任务目录与确定性 runbook 引擎（固定决策流，readonly only）。
- 异步 job 模型：`agent_run_task` 返回 job id、`agent_job_status`、`agent_list_tasks`。
- 结果存为 MCP resource（如 `roswire://diagnostics/<job-id>`），含结构化 JSON、人类摘要与完整 tool-call trace。
- job 注册表 + 持久化（`ROSWIRE_HOME` 下，含 TTL/清理/重启语义）、并发 job 上限、result size limit（复用 9.4）。

完成标准：

- runbook 输出可被 `cargo test` 断言（确定性）。
- 全程 readonly，永不调用写/文件/config-mutation 工具。
- 不依赖 OpenAI；构建不引入 LLM 依赖。

### Phase 8：实验性 LLM Agent / ReAct 层（P3，默认关闭）

> 设想里"大胆"的一半。作为 Phase 7 之上的**可选推理层**，feature-gated、默认关闭。详见第 14 节。

产出：

- `--features agent` 下的 ReAct/plan-then-execute 推理层，复用 Phase 7 的 job/resource 管道。
- `[agent]` 配置段与 OpenAI-兼容 client（`base_url` 可配，key 走 secret 后端）。
- 步数/token/时长/花费/并发上限与 cancellation。

完成标准：

- 默认 `enabled=false`；纯 CLI 与只读 MCP 用户零 OpenAI 依赖、零体积负担。
- agent 仅能 readonly，工具 allowlist 收窄，全部决策与工具调用进审计日志。
- 设备数据出域、脱敏与端点配置在文档中明确告知。

## 12. 主要风险

### 12.1 万能工具诱导越权

如果只暴露 `execute(tokens)`，模型会把它当作通用命令执行器。第一版必须优先提供语义化 tools，并把通用 command tool 限制在 readonly。

### 12.2 raw 写入边界过宽

现有 `--allow-write` 是显式逃生口，不适合直接暴露给 MCP。MCP 第一版不得提供该参数。

### 12.3 配置 mutation 变成远程提权入口

如果 MCP 可以修改 profile 或 secret，就可能间接扩大 RouterOS 权限。第一版只读 server 不开放 config write 和 secret write。

### 12.4 结果过大拖垮 client

RouterOS 某些 print 结果可能很大。MCP server 必须支持 result size limit，并在超限时返回结构化错误。

### 12.5 同步栈下超时无法真正中断

`ureq`/`ssh2` 阻塞调用不可被外层 timer 中断，孤立线程会跑到底层 socket 超时。必须配底层超时 + 有界阻塞线程池，否则高频或卡死调用会耗尽线程。见 7.2。

### 12.6 panic 杀掉常驻进程

`panic = "abort"` 下任一 tool panic 会终止整个 server。进入实现前必须按 7.3 选定隔离方案，并在 `spawn_blocking` 边界做 `catch_unwind`。

### 12.7 明文 HTTP 暴露 token

面向远程意味着 bearer token 必须在 TLS 之上。直接把明文 HTTP+bearer 暴露到不可信网络属于误配；`server_info` 须反映 TLS 状态，文档须明确部署要求。见 9.3。

## 13. Go / No-Go 清单

进入实现前：

- [ ] tool 命名和风险等级确认。
- [ ] 默认 policy 确认只读。
- [ ] executor 抽象边界确认。
- [ ] 认证最小方案确认。
- [ ] 传输确认为 Streamable HTTP，MCP SDK 与 async/HTTP 栈选定并 pin。
- [ ] panic 隔离方案（7.3）确认。
- [ ] 超时双层语义（7.2）确认。
- [ ] 远程部署 TLS 要求（9.3）写入文档。

Phase 4 合并前：

- [ ] CLI 回归测试通过。
- [ ] MCP server 单元/集成测试通过。
- [ ] raw 非 print 在 MCP 下无法调用。
- [ ] 写操作在 catalog 中状态为 disabled，调用返回 policy error。
- [ ] secret 脱敏测试覆盖 MCP result。
- [ ] 单请求 panic 不杀进程、tool timeout 返回结构化错误已验证。

## 14. 实验性 Agent 模式（P3 / 默认关闭）

> 优先级：**P3，实验性，默认关闭**。仅在只读 MCP server（Phase 3–5）稳定后启动；不在第一版承诺，不进入默认构建。

### 14.1 定位与拆分

设想是：在 roswire 内实现一个 ReAct agent，跑预设诊断任务，把结果存为 MCP resource，client 下发任务、稍后取结果。评估后**拆成两层**，价值与风险差异很大：

- **Phase 7（P2，确定性）**：预设 runbook + job/resource 管道，**不需要 LLM**。固定排障流程用确定性决策树实现，更可靠、可测、免费、可复现。大部分价值在这一层。
- **Phase 8（P3，LLM）**：在 runbook 之上叠 ReAct/LLM 推理层，处理开放式归纳与自适应选步。这才需要 OpenAI 配置，也是风险集中点。

原则：**先把确定性的一半做实并证明价值，再把 LLM 作为可选层叠加。**

### 14.2 契合点（为什么可行）

- 复用同一 executor + policy：agent 只是又一个调用方，照走只读门禁、mapping、raw safety、脱敏。强制 readonly + 工具 allowlist + 永不给写工具后，爆炸半径被现有边界框住。
- 复用 tokio：Phase 3 已引入 async/HTTP 运行时，agent 后台 job 与 LLM 异步调用边际成本低。
- 复用 secret 后端：OpenAI key 走现有 keychain/secret 体系，不落明文 config。
- MCP resources 是结果存储的正确原语。

### 14.3 配置草案

```toml
[agent]
enabled = false
provider = "openai_compatible"
base_url = "https://api.openai.com/v1"   # 可指向 Azure / 自托管 / 本地模型
model = "gpt-4o-mini"
temperature = 0
max_steps = 8
max_tokens = 4096
max_wall_clock_seconds = 120
max_concurrent_jobs = 2
redact_device_data = true

[agent.secrets.api_key]
type = "keychain"
service = "roswire"
account = "agent/openai_api_key"
```

### 14.4 主要风险（集中在 Phase 8）

- **确定性 / 身份冲突**：roswire 卖点是 JSON-first、确定、可测；LLM 输出不确定，与精确 payload 断言冲突。→ LLM 输出隔离在 agent 模块，核心工具语义保持确定。
- **数据出域**：设备拓扑、地址、防火墙规则、邻居 identity 会发往 LLM 提供方。→ 默认禁用、显式 opt-in、`base_url` 可配、可配脱敏、文档明确告知数据流向。
- **Prompt injection**：RouterOS 输出（comment、DNS 名、日志、identity）进入 LLM 上下文可注入指令。→ 只读门禁兜底；进一步收窄候选工具集、限定步数，优先 plan-then-execute 而非无界 ReAct。
- **跑飞的成本/循环**：ReAct 自循环烧 token。→ 14.3 的 step/token/时长/花费/并发硬上限 + cancellation。
- **scope 蠕变**：本特性把"异步任务投递"重新引入（原属非目标）。→ 单独立项、feature-gated，不混入只读 server。
- **build 膨胀**：拉进异步 HTTP + tool-calling schema。→ `--features agent` 默认关，保证非 agent 用户零负担、零 OpenAI 依赖。

### 14.5 实验性 Go / No-Go（Phase 8 启动前）

- [ ] Phase 7 的确定性 runbook + job/resource 已稳定。
- [ ] agent 仅 readonly、工具 allowlist 已冻结。
- [ ] OpenAI-兼容端点、key 走 secret、默认 `enabled=false` 已确认。
- [ ] step/token/时长/花费/并发上限与 cancellation 已实现。
- [ ] 数据出域与脱敏策略、prompt injection 缓解已评审。

## 15. 最终结论

基于 Streamable HTTP 的远程 MCP server 是 `roswire` 服务化面向 AI client 的目标形态。它复用现有 CLI、mapping、protocol、config 和 error 体系，同时给远程 AI client 一个标准工具接口；本机使用仍走 CLI（由 skill 封装），不经过 MCP。

第一版的关键不是开放更多写能力，而是把"已有功能可发现、可调用、可审计、可裁剪"这条链路做稳。写操作应在只读 server 稳定后，以单 tool allowlist 的形式逐步开放。

内嵌 agent 模式（第 14 节）是一个**低优先级、默认关闭**的实验方向：先做确定性诊断 runbook + job/resource 管道（P2，无 LLM），再把 ReAct/LLM 作为可选、只读、端点可配、成本受限的推理层叠加（P3）。它不影响、也不阻塞只读 server 主线。
