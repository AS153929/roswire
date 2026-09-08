# RosWire 功能 Checklist

> 最后更新：2026-06-01
> 基准分支：`main`
> 已创建 backlog issues：`#60`-`#76`
> 关联规划：[`docs/roadmap.md`](roadmap.md)（里程碑视角）、[`docs/mcp-server-development-plan.md`](mcp-server-development-plan.md)

本文是 `roswire` 的**功能总账与最终目标定义**，作用有二：

1. **追踪完成度**：已交付能力用 `[x]` 标记；未完成或仍需真机验证/扩展的项用 `[ ]` 标记。
2. **锚定最终目标、防止开发偏离**：清单同时收录"已交付"与"规划中"的功能。任何开发只应推进清单内的项；清单外的新增能力，必须先回到本文与开发计划评审、确认优先级后再实现，避免"顺手扩张"导致范围漂移。

清单分两部分：

- **第一部分：已交付能力** —— 反映当前 `main` 的真实实现。
- **第二部分：规划能力（尚未实现）** —— 来自开发计划，按 **P0=必做地基 / P1=第一版目标 / P2=后续增强 / P3=实验性默认关闭** 标注优先级，并标注对应 Phase。规划项在落地前一律保持 `[ ]`。

---

# 第一部分：已交付能力

## 总览

- [x] MVP 规划 issue 队列完成并合并到 `main`
- [x] 本地 `main` 与 `origin/main` 同步
- [x] 核心 CLI / JSON 契约完成
- [x] 配置、密钥、错误模型、自描述与诊断完成
- [x] RouterOS API / API-SSL / REST 核心执行链路完成
- [x] SSH/SFTP 文件上传下载完成
- [x] import / export / backup 文件工作流完成
- [x] JSONL 日志、保留策略与脱敏 debug 完成
- [ ] 生产级真机矩阵验证（#60；已提供 harness/矩阵文档，待真实 RouterOS/CHR 记录）
- [ ] 更大范围 RouterOS 命令覆盖
- [x] 发布打包与安装说明（#61）
- [x] 生产级稳定版验收门槛定义（#63）

## CLI 与输出契约

- [x] `roswire [global-options] <path...> <action> [key=value ...]` 基础命令形态
- [x] 成功结果只写入 `stdout`
- [x] 错误结果以结构化 JSON 写入 `stderr`
- [x] 非零退出码与稳定错误码映射
- [x] `--json` 输出机器可读 JSON
- [x] `--debug` 输出脱敏诊断信息
- [x] 默认输出避免随机 ID、时间戳、耗时等不稳定字段
- [x] `BTreeMap` / 结构体保证关键 JSON 字段顺序稳定
- [x] 缺少必要参数时立即失败，不进入交互式询问

## 全局参数与配置来源

- [x] `--profile` / `default_profile` / 单 profile 推导
- [x] `--host` / profile `host`
- [x] `--user` / profile `user`
- [x] `--password` / profile secret `password`
- [x] `--protocol` / profile `protocol`
- [x] `--routeros-version` / profile `routeros_version`
- [x] `--port` / profile `port`
- [x] `--transfer` / profile `transfer`
- [x] `--ssh-port` / profile `ssh_port`
- [x] `--ssh-user` / profile `ssh_user`
- [x] `--ssh-password` / profile secret `ssh_password`
- [x] `--ssh-key` / profile `ssh_key`
- [x] profile secret `ssh_key_passphrase`
- [x] `--ssh-host-key` / profile `ssh_host_key`
- [x] `--allow-from` / profile `allow_from`
- [x] `--ensure-ssh`
- [x] `--restore-ssh`
- [x] 配置优先级：CLI 参数 > profile > 默认值
- [x] 移除设备级 `ROS_*` 环境变量入口，保留 `ROSWIRE_HOME` / `ROSWIRE_DEBUG` / secret 后端变量
- [x] `auto` 协议下禁止单一 `--port` 覆盖

## 本地配置与 profile

- [x] `ROSWIRE_HOME` 覆盖默认工作目录
- [x] `config init`
- [x] `config inspect`
- [x] `config profiles`
- [x] `config device add`
- [x] `config device set`
- [x] `config secret set`
- [x] `secret set` alias
- [x] `~/.roswire/config.toml` 读写
- [x] `~/.roswire/logs/` 路径管理
- [x] Unix/macOS 目录权限目标 `0700`
- [x] Unix/macOS config 权限目标 `0600`
- [x] 配置权限过宽时返回结构化错误
- [x] profile 不存在时返回 `PROFILE_NOT_FOUND`
- [x] 未选择 profile 时按 CLI / default_profile / 单 profile 推导
- [x] 拒绝把 MAC 地址作为 RouterOS `host`
- [x] `config inspect` 输出 resolved 字段来源
- [x] `config inspect` 输出 logging 配置
- [x] `config inspect` 对 secret 与本地敏感路径脱敏

## Secret 管理

- [x] `plain` secret
- [x] `encrypted` secret
- [x] `keychain` secret
- [x] `env` secret
- [x] `same-as` secret
- [x] 明文 secret 需要 `allow_plain_secrets = true`
- [x] encrypted secret 使用环境 master key 解密
- [x] keychain 后端错误映射为结构化错误
- [x] env secret 只保存环境变量名，不保存实际值
- [x] same-as 循环检测
- [x] `--stdin` secret 输入
- [x] secret inspect / config inspect 不泄露真实值
- [x] 加密私钥 passphrase 非交互支持（profile secret `ssh_key_passphrase`）
- [x] 多平台 keychain smoke test 矩阵（#62：PR documented fallback，macOS 原生 smoke 走 workflow_dispatch/本地验证）

## RouterOS 命令映射

- [x] `interface print`
- [x] `system resource print`
- [x] `ip address print`
- [x] `ip address add`
- [x] `ip address set`
- [x] `ip address remove`
- [x] CLI path/action 到 RouterOS classic API 路径映射
- [x] CLI path/action 到 RouterOS REST 路径映射
- [x] `key=value` 参数解析
- [x] 写操作返回稳定 `roswire.write.v1` payload
- [x] 不支持的命令/action 返回 `UNSUPPORTED_ACTION`
- [x] `/ip/firewall` 命令族（address-list/filter/nat print，#70）
- [x] `/ip/route` 命令族（print，#71）
- [x] `/interface/wireguard` 命令族（interface/peers print，#72）
- [x] `/system/package` 命令族（print，#73）
- [x] `/user` 命令族（print，#74）
- [x] `/tool` 命令族（netwatch/mac-server print，#75）
- [x] 原始命令透传模式（显式 `raw`、classic API、写操作需 `--allow-write`，#69）
- [x] `script put <name> --source @<local.rsc>` 工作流（#68）

## 协议层

- [x] classic RouterOS API word/sentence 编解码
- [x] `!re` / `!done` / `!trap` / `!fatal` 解析
- [x] TCP API transport
- [x] TLS API-SSL transport
- [x] modern login
- [x] v6 challenge-response login
- [x] `/system/resource/print` 版本探测
- [x] REST client
- [x] REST GET
- [x] REST PUT
- [x] REST PATCH
- [x] REST DELETE
- [x] REST POST JSON
- [x] REST 空响应成功处理
- [x] HTTP 状态码到结构化错误映射
- [x] RouterOS trap/error 到结构化错误映射
- [x] 认证失败不静默回落
- [x] 网络错误可进入下一个 auto 候选协议
- [ ] 真实设备 TLS 证书异常矩阵验证
- [ ] RouterOS v6/v7 字段差异大范围验证

## 协议自动选择

- [x] `--protocol auto` 默认模式
- [x] REST 候选探测
- [x] API-SSL 候选探测
- [x] API 候选探测
- [x] REST 可用且当前动作有 REST 映射时优先 REST
- [x] REST 不可用时回落 classic API
- [x] 当前动作无 REST 映射时回落 classic API
- [x] 显式 `api` / `api-ssl` / `rest` 不被自动改道
- [x] selected protocol 写入错误上下文
- [x] requested protocol 写入错误上下文

## Agent 自描述与诊断

- [x] `commands --json`
- [x] `commands --remote --json`
- [x] `help --json`
- [x] `help <command...> --json`
- [x] `schema command <command...> --json`
- [x] `schema command <command...> --remote --json`
- [x] `schema output <command...> --json`
- [x] `schema discover --remote --json`
- [x] `doctor --json`
- [x] `doctor --include-remote --json`
- [x] `explain-error <code> --json`
- [x] 默认自描述命令不访问 RouterOS
- [x] 远端 schema overlay 支持降级输出
- [x] 远端 schema cache key 模型
- [x] cache 中不写入 secret
- [x] doctor 本地配置/权限/依赖检查
- [x] doctor 远端协议与错误分类检查
- [x] schema cache TTL / refresh 策略完整产品化（#64）
- [x] 更多 RouterOS 菜单的远端字段/枚举覆盖（静态字段 + runtime hint 来源标记，#64）

## SSH/SFTP 文件传输

- [x] `file upload <local> <remote>` dry-run plan
- [x] `file download <remote> <local>` dry-run plan
- [x] `file upload <local> <remote>` 真实 SSH/SFTP 上传
- [x] `file download <remote> <local>` 真实 SSH/SFTP 下载
- [x] SSH host key fingerprint 必填与校验
- [x] SSH password auth
- [x] SSH key auth
- [x] 加密 SSH key passphrase 非交互解析
- [x] SSH 用户/密码可与 API 控制面凭据分离
- [x] 默认复用 API 用户/密码作为 SSH 凭据
- [x] SFTP 数据面
- [x] SFTP 不可用时 SCP fallback
- [x] SHA256 checksum
- [x] 64 MiB 文件大小限制
- [x] 本地绝对路径脱敏
- [x] SSH 私钥路径脱敏
- [x] 缺少 host key 返回 `SSH_HOST_KEY_REQUIRED`
- [x] host key 不匹配返回 `SSH_HOST_KEY_MISMATCH`
- [x] 文件过大返回 `FILE_TOO_LARGE`
- [x] 传输失败返回 `FILE_TRANSFER_FAILED`
- [x] SCP fallback（SFTP subsystem 打不开时尝试 SCP）
- [x] 加密 SSH 私钥 passphrase 支持（env/profile secret；不新增 passphrase CLI 明文参数）
- [ ] 真实 RouterOS SFTP/SCP 兼容矩阵验证

## 文件工作流

- [x] `import <local.rsc>` dry-run plan
- [x] `import <local.rsc>` 真实编排
- [x] `export download <local.rsc>` dry-run plan
- [x] `export download <local.rsc>` 真实编排
- [x] `backup download <local.backup>` dry-run plan
- [x] `backup download <local.backup>` 真实编排
- [x] import 上传临时文件后执行 `/import file-name=<temp>`
- [x] export 执行 `/export file=<name>` 后等待 `.rsc`
- [x] export 支持 `--compact`
- [x] backup 执行 `/system/backup/save name=<name>` 后等待 `.backup`
- [x] 下载先写入 `.part`，成功后 finalize 到目标路径
- [x] 可选 cleanup 临时文件
- [x] cleanup 失败返回结构化错误
- [x] 控制面支持 classic API
- [x] 控制面支持 REST POST JSON
- [x] 数据面复用 SSH/SFTP
- [x] 覆盖策略可配置化（#66）
- [x] 超时/重试策略更细粒度配置（#66）
- [ ] 真实设备 import/export/backup 端到端验证

## SSH 服务准备与白名单

- [x] `--ensure-ssh` CLI 参数
- [x] `--restore-ssh` CLI 参数
- [x] `--allow-from` CLI/profile 参数
- [x] dry-run 中声明 SSH 前置条件
- [x] CIDR 安全校验
- [x] 拒绝过宽 IPv4 白名单
- [x] 拒绝过宽 IPv6 白名单
- [x] 缺少白名单时返回结构化错误
- [x] 真实执行 `/ip service ssh` enable / address set
- [x] 任务前 SSH 服务快照
- [x] 成功路径 restore SSH 服务配置
- [x] 失败路径 restore SSH 服务配置
- [ ] 中断路径自动 restore（当前在 dry-run 明确限制：不捕获进程中断）
- [x] 白名单追加/合并现有地址而非覆盖

## SSH 跳板传输（L0）

> 评审结论：跳板是一次 CLI 调用内的 `direct-tcpip` 传输适配。不引入 daemon、不本地 bind、不修改 sshx。

- [x] profile `[[jump]]` 多跳配置与 CLI `--jump-*` 单跳覆盖
- [x] `config device add/set` 支持 `jump_host` / `jump_port` / `jump_user` / `jump_key` / `jump_host_key`
- [x] `config inspect` 输出跳板身份（私钥路径脱敏）
- [x] 缺跳板 host key 返回 `JUMP_HOST_KEY_REQUIRED`
- [x] 跳板 host key 不匹配返回 `JUMP_HOST_KEY_MISMATCH`
- [x] 拆除失败返回 `JUMP_TEARDOWN_FAILED` 且不得把命令成功当作调用成功
- [x] `plan_dials`：首跳 TCP connect，后续与目标为 `direct-tcpip`；无本地监听
- [x] API / API-SSL / REST 经同一跳板到达目标管理端口
- [x] SFTP/SCP 经跳板到达 RouterOS SSH
- [x] Drop / 显式 close 拆除 channel 与 session
- [x] `--dry-run` 命令计划与 transfer plan 含 `via.jump`（`local_bind=false`，`will_connect=false`）
- [x] JSONL `jump.opened` / `jump.closed` / `jump.failed`，secret 脱敏
- [x] `doctor` 报告跳板配置；`--include-remote` 分腿报告 bastion 与 target
- [ ] 真机双跳 / 多跳验收记录

## JSONL 日志与调试

- [x] `[logging] enabled`
- [x] `[logging] retention_days`
- [x] `[logging] level`
- [x] 默认日志开启
- [x] 可关闭日志且不创建日志文件
- [x] 日志路径 `.roswire/logs/roswire-YYYY-MM-DD.log`
- [x] JSON Lines，每行一个事件
- [x] 启动时执行保留期清理
- [x] 最大保留 30 天
- [x] 清理失败不阻塞主命令
- [x] `--debug` 提高本次运行诊断详细度
- [x] debug 输出不污染 `stdout`
- [x] 日志与 debug 脱敏 password/secret/token
- [x] 日志与 debug 脱敏 SSH key path
- [x] 日志与 debug 脱敏本地绝对路径
- [x] logging 初始化不隐式创建缺失的 `.roswire`，保持 doctor 语义

## 错误模型

- [x] `RosWireError` 结构化 JSON
- [x] `ErrorContext` 上下文
- [x] `USAGE_ERROR`
- [x] `CONFIG_ERROR`
- [x] `PROFILE_NOT_FOUND`
- [x] `CONFIG_INSECURE_PERMISSIONS`
- [x] `SECRET_BACKEND_UNAVAILABLE`
- [x] `SECRET_NOT_FOUND`
- [x] `SECRET_DECRYPT_FAILED`
- [x] `AUTH_FAILED`
- [x] `NETWORK_ERROR`
- [x] `ROS_API_FAILURE`
- [x] `UNSUPPORTED_ACTION`
- [x] `SSH_SERVICE_UNAVAILABLE`
- [x] `SSH_HOST_KEY_REQUIRED`
- [x] `SSH_HOST_KEY_MISMATCH`
- [x] `SSH_WHITELIST_REQUIRED`
- [x] `SSH_WHITELIST_UNSAFE`
- [x] `SSH_RESTORE_FAILED`
- [x] `JUMP_HOST_KEY_REQUIRED`
- [x] `JUMP_HOST_KEY_MISMATCH`
- [x] `JUMP_TEARDOWN_FAILED`
- [x] `FILE_TOO_LARGE`
- [x] `FILE_TRANSFER_FAILED`
- [x] `SERIALIZATION_ERROR`
- [x] `HELP_TOPIC_NOT_FOUND`
- [x] `SCHEMA_UNAVAILABLE`
- [x] `REMOTE_SCHEMA_UNAVAILABLE`
- [x] `CAPABILITY_PROBE_FAILED`
- [x] `REMOTE_SCHEMA_STALE`
- [x] `INTERNAL_ERROR`
- [x] sensitive key/value 脱敏
- [x] resolved args 脱敏

## 测试与质量门

- [x] 单元测试覆盖 CLI / mapping / protocol / config / transfer / logging
- [x] 集成测试覆盖 CLI smoke
- [x] 集成测试覆盖 config read/write
- [x] 集成测试覆盖 introspect static
- [x] 集成测试覆盖 transfer dry-run
- [x] `cargo fmt --check`
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [x] `cargo test --workspace --all-features`
- [x] `cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 85`
- [x] 覆盖率门槛 85%
- [ ] RouterOS CHR / 真机集成测试流水线（#60；harness 已提供，等待真实目标/凭据）
- [ ] Linux / Windows 发布 smoke test（macOS artifact 暂缓，等待 runner 或交叉编译方案）
- [x] keychain 多平台 smoke test（#62）

## 文档与发布

- [x] `README.md` 基础说明
- [x] `docs/develop-plan.md` 开发规格与阶段计划
- [x] `docs/feature-checklist.md` 功能完成度 checklist
- [x] `docs/production-readiness.md` 生产级稳定版验收门槛（#63）
- [x] `docs/installation.md` 安装、校验、卸载说明（#61）
- [x] `docs/release.md` 维护者发布流程（#61）
- [x] `docs/routeros-acceptance-matrix.md` 真机/CHR 验收矩阵与 harness 说明（#60 准备工作）
- [x] README 与当前实现的完整示例校正（#61）
- [x] 安装说明（#61）
- [x] GitHub Releases 发布流程（#61）
- [x] 二进制校验和（#61）
- [x] 平台打包说明（#61）

---

# 第二部分：规划能力（尚未实现）

> 来源：[`docs/mcp-server-development-plan.md`](mcp-server-development-plan.md)。优先级：**P0 必做地基 / P1 第一版目标 / P2 后续增强 / P3 实验性默认关闭**。落地前一律 `[ ]`。
>
> 边界守则：第二部分定义 `roswire` 服务化的**最终目标范围**。MCP 第一版只读、写操作默认禁用、agent 模式默认关闭——这些是不可在开发中途擅自突破的红线。

## 执行核心抽象（Phase 2）

- [ ] (P0) executor 抽象：执行逻辑与 `stdout`/`&Cli` 解耦，返回结构化 payload 而非直接打印
- [ ] (P0) CLI 改为 executor 薄包装，输出/错误码/脱敏与现状完全一致
- [ ] (P0) executor 可被非 CLI（MCP/agent）调用，复用 mapping、raw safety、协议层与错误模型
- [ ] (P0) `ExecutionMode`：Readonly / WriteAllowed / ConfigAllowed / TransferAllowed

## 配置 include（Phase 1）

- [ ] (P1) 主配置 `include`（相对路径 + 简单 glob，禁绝对路径/`..`/软链逃逸）
- [ ] (P1) 每 profile 一文件：设备连接 + secret + 该 profile 的 MCP policy 同文件
- [ ] (P1) include 文件权限检查（`0600`/目录 `0700`）
- [ ] (P1) include 文件只允许补充同名 profile 子表，全局字段仍限主配置
- [ ] (P1) 稳定展开顺序、重复 profile/policy 报错、单文件至多一个设备 profile
- [ ] (P1) `config inspect` / `mcp server_info` 输出每个 profile 的脱敏来源文件
- [ ] (P1) include 相关错误码（不存在/无匹配/权限过宽/重复/非法路径/非法全局字段/名称不一致）

## MCP server 骨架与运行时（Phase 3）

- [ ] (P0) `roswire mcp serve`，**仅 Streamable HTTP** 传输（不实现 stdio）
- [ ] (P0) `tokio` + HTTP 栈 + MCP SDK（候选 `rmcp`）接入并 pin 版本
- [ ] (P0) 同步执行经 `spawn_blocking` 有界线程池，不在 async 任务里阻塞 I/O
- [ ] (P0) 并发上限 + `server_busy` 结构化错误
- [ ] (P0) 双层超时：socket 级 + MCP 层 `tool_timeout`，且层间满足 ≥ 关系
- [ ] (P0) panic 隔离：单请求 panic 不杀进程，`spawn_blocking` 边界 `catch_unwind`
- [ ] (P0) `[mcp]` 配置段（bind/auth/默认 readonly/size/timeout）与 per-profile policy
- [ ] (P0) bearer token 认证；非 loopback 无 token 拒绝启动
- [ ] (P0) loopback 无 token 仅联调，标记 `auth_mode=loopback_dev`
- [ ] (P1) TLS 由前置反代终止；`server_info` 反映 `tls=external|none`
- [ ] (P1) result size limit，超限返回 `result_too_large`
- [ ] (P1) MCP 调用审计日志（profile/tool/risk/policy 判定/耗时/结果大小，复用 logging，secret 不入日志）

## MCP 工具：基础与自描述（Phase 3–5）

- [ ] (P1) `roswire_server_info`
- [ ] (P1) `roswire_list_profiles`
- [ ] (P1) `roswire_profile_inspect`（脱敏连接配置与 secret 状态）
- [ ] (P1) `roswire_list_tools`（policy 裁剪后的 catalog）
- [ ] (P1) `roswire_doctor`（可选 include remote）
- [ ] (P1) `roswire_commands`
- [ ] (P1) `roswire_help`
- [ ] (P1) `roswire_schema`
- [ ] (P1) `roswire_explain_error`
- [ ] (P1) secret/password/private key 路径不出现在任何 tool result 中

## MCP 工具：RouterOS 只读（Phase 4）

- [ ] (P1) `routeros_observe_interfaces`
- [ ] (P1) `routeros_observe_addresses`
- [ ] (P1) `routeros_observe_routes`
- [ ] (P1) `routeros_observe_firewall`
- [ ] (P1) `routeros_observe_wireguard`
- [ ] (P1) `routeros_observe_system`
- [ ] (P1) 通用只读 command tool（`action` 仅 `print`，拒绝 file/interval/follow）
- [ ] (P1) `routeros_raw_print`（path 末段必须 `print`、经 `normalize_raw_routeros_path()`、可选 allowlist）
- [ ] (P1) 所有只读工具经 MCP policy + roswire mapping 双重校验
- [ ] (P1) 返回保留 `selected_protocol`、错误码与脱敏上下文

## MCP 权限策略与风险分级（Phase 3+）

- [ ] (P0) risk taxonomy：readonly / local_config_read / local_config_write / routeros_write / connectivity_risk / secret_sensitive / file_transfer
- [ ] (P0) 全量 catalog + policy 裁剪：高风险 tool 可见但返回 `disabled_by_policy`
- [ ] (P0) 默认只读；写、文件、config-mutation 默认禁用
- [ ] (P0) 双重校验：任一层拒绝都不连接 RouterOS
- [ ] (P0) server 侧独占授权：client 不能选 host/凭据/raw 写/文件导入

## MCP 写与文件工具（Phase 6，P2，默认禁用）

- [ ] (P2) `routeros_write_*`（disabled_by_policy，profile 显式开启 + tool allowlist）
- [ ] (P2) `routeros_raw_write`（单独显式开启，不继承普通写权限）
- [ ] (P2) `roswire_file_upload`（显式开启 transfer policy）
- [ ] (P2) `roswire_file_download`（限制路径与大小）
- [ ] (P2) `roswire_import` / `roswire_backup_download`（后续阶段，需 dry-run 与人工确认）
- [ ] (P2) safe-write policy / dry-run first / per-tool allowlist

## 实验性 Agent 模式：确定性诊断（Phase 7，P2）

> 详见开发计划第 14 节。这一层不引入 LLM。

- [ ] (P2) 预设诊断任务目录 + 确定性 runbook 引擎（全程 readonly）
- [ ] (P2) 异步 job 模型：`agent_run_task`（返回 job id）/ `agent_job_status` / `agent_list_tasks`
- [ ] (P2) 结果存为 MCP resource（`roswire://diagnostics/<job-id>`，含结构化 JSON + 摘要 + tool-call trace）
- [ ] (P2) job 注册表 + 持久化（`ROSWIRE_HOME` 下，含 TTL/清理/重启语义）+ 并发 job 上限
- [ ] (P2) runbook 输出可被 `cargo test` 断言（确定性），不依赖 OpenAI

## 实验性 Agent 模式：LLM ReAct 层（Phase 8，P3，默认关闭）

> 详见开发计划第 14 节。`--features agent`，默认关闭，仅在只读 MCP 稳定后启动。

- [ ] (P3) ReAct / plan-then-execute 推理层，复用 Phase 7 的 job/resource 管道
- [ ] (P3) `[agent]` 配置段 + OpenAI 兼容 client（`base_url` 可配，key 走 secret 后端）
- [ ] (P3) step / token / 时长 / 花费 / 并发上限 + cancellation
- [ ] (P3) 默认 `enabled=false`；纯 CLI 与只读 MCP 用户零 OpenAI 依赖、零体积负担
- [ ] (P3) agent 仅 readonly + 工具 allowlist 收窄 + 永不暴露写/文件/config-mutation 工具
- [ ] (P3) 全部 LLM 决策与工具调用进审计日志
- [ ] (P3) 数据出域、脱敏与 prompt injection 缓解已评审

---

# 第三部分：当前判断与 backlog

## 当前判断

- [x] MVP 功能闭环完成
- [x] MVP 规划 issue 队列清零
- [x] 可以进入 Beta 试用与真机验收
- [ ] 可以声明生产级稳定版
- [ ] MCP 服务化第一版（只读）落地（规划，见第二部分）
- [ ] 实验性 agent 模式落地（规划，P3，默认关闭）

## 下一阶段建议

- [x] 创建真机验收 issue：RouterOS v6 / v7 / REST / API / API-SSL / SSH/SFTP 矩阵（#60）
- [x] 创建命令覆盖扩展 issue：优先 `/ip/firewall`、`/ip/route`、`/interface/wireguard`（#70-#75）
- [x] 创建发布工程 issue：release workflow、checksum、安装文档（#61）
- [x] 清理旧防御性文案，避免误导 SSH transfer 运行时状态（#76）

## Backlog issue 对照

- #60 `M8: 建立 RouterOS 真机/CHR 验收矩阵`（准备完成：`scripts/routeros-acceptance.sh`、`docs/routeros-acceptance-matrix.md`；仍需真实 RouterOS/CHR 运行记录后关闭）
- #61 `M8: 发布工程与安装文档`（完成：Windows artifact、release smoke、checksum、安装/发布文档）
- #62 `M8: 多平台 keychain smoke 测试`（完成：`tests/keychain_smoke.rs` ignored smoke、CI `keychain-smoke` job、`docs/keychain-smoke.md` 平台依赖与 fallback）
- #63 `M8: 定义生产级稳定版验收门槛`（完成：MVP/Beta/Production 边界、P0 blockers、质量门、发布物与安全门槛）
- #64 `M7: 扩展远端 schema cache TTL/refresh 与菜单 overlay`（完成：hit/miss/stale/refresh 决策、`--refresh`、overlay enum 来源标记）
- #65 `M7: 实现 SSH 服务 ensure/restore 与白名单合并`（完成：SSH service 快照、enable/address 合并、成功/失败 restore、`SSH_RESTORE_FAILED`）
- #66 `M7: 增强文件工作流覆盖策略、超时与重试`（完成：`--if-exists`、transfer timeouts、有限 retry、dry-run policy）
- #67 `M7: 实现 SCP fallback 与加密 SSH 私钥 passphrase 支持`（完成：SFTP subsystem 不可用时 SCP fallback、profile secret `ssh_key_passphrase`、脱敏与 dry-run 标记；真实设备运行记录仍是 production-stable 门槛）
- #90 `移除单设备 ROS_* 环境变量配置入口`（完成：设备/连接/传输字段优先级收敛为 CLI > profile > defaults，保留 `ROSWIRE_*` 全局/secret 后端变量）
- #68 `M7: 实现 system script put 工作流`（完成：dry-run、UTF-8/大小校验、classic/REST 写入映射与脱敏）
- #69 `M7: 实现 RouterOS raw command passthrough`（完成：显式 raw、classic words、只读/写安全边界、REST raw 不开放说明）
- #70 `M7: 扩展 /ip/firewall 命令族`（完成：address-list/filter/nat print）
- #71 `M7: 扩展 /ip/route 命令族`（完成：print）
- #72 `M7: 扩展 /interface/wireguard 命令族`（完成：interface/peers print）
- #73 `M7: 扩展 /system/package 命令族`（完成：print）
- #74 `M7: 扩展 /user 命令族`（完成：print）
- #75 `M7: 扩展 /tool 命令族`（完成：netwatch/mac-server print）
- #76 `Cleanup: 审计 unsupported/not implemented 文案与 checklist 同步`
