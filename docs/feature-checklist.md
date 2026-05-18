# RosWire 功能 Checklist

> 最后更新：2026-05-18
> 基准分支：`main`
> 已创建 backlog issues：`#60`-`#76`

本文用于快速追踪 `roswire` 的功能完成度。已完成项使用 `[x]` 标记；未完成或仍需真机验证/扩展的项使用 `[ ]` 标记。

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

## 当前判断

- [x] MVP 功能闭环完成
- [x] MVP 规划 issue 队列清零
- [x] 可以进入 Beta 试用与真机验收
- [ ] 可以声明生产级稳定版

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
