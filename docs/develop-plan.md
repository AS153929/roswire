# RosWire 开发计划

本文是 `roswire` 的实现规格与阶段计划。用户入口、安装方式和使用示例见 [`../README.md`](../README.md)。

## 1. 项目目标

`roswire` 是面向 AI Agent 与自动化脚本的 RouterOS CLI 桥接工具。它不追求人类交互体验，而是优先保证输出稳定、错误可解析、调用可组合。

### 目标

- 提供非交互、JSON 优先（JSON-first）、可脚本化的 RouterOS 操作入口。
- 支持 RouterOS 原生 API 的 v6/v7 方言，以及 RouterOS v7 REST API。
- 严格隔离 `stdout` 与 `stderr`，便于 Agent 运行时捕获结果与错误。
- 产出稳定错误结构，帮助 Agent 根据错误上下文自我修正。
- 编译为单个原生二进制文件，不依赖外部运行时。

### 非目标

- 不替代 WinBox、WebFig 或 RouterOS 交互式终端。
- 不在 CLI 内保存长期会话或连接池。
- 不默认输出彩色文本、加载动画、进度条或分页器（pager）。
- 不在命令行交互式询问密码、确认操作或二次输入。

## 2. 核心设计原则

### 无状态执行

每次执行都是一次独立的 TCP/TLS/HTTP 生命周期。CLI 不假设上一次调用留下的本地状态。

### 确定性输出

在相同输入与相同 RouterOS 设备状态下，默认输出应尽量保持字节级稳定。

实现约束：

- 默认错误载荷不包含时间戳（timestamp）、随机 ID、耗时等运行时变化字段。
- JSON 对象中需要稳定顺序的字段使用结构体或 `BTreeMap`，避免直接序列化 `HashMap`。
- 调试信息必须显式打开，例如 `--debug`；默认不能污染 `stdout`。

### 流隔离

- `stdout`：只输出成功结果载荷。
- `stderr`：只输出诊断、调试信息与错误载荷。
- 所有错误都以单个 JSON 对象写入 `stderr`，并以非零状态码退出。

### 零交互

如果缺少必要参数，立即输出结构化错误并退出。禁止出现 `Are you sure? [y/n]`、密码提示或任何等待用户输入的流程。

## 3. CLI 与配置约定

### 命令格式

```text
roswire [global-options] <path...> <action> [key=value ...]
```

示例：

```bash
roswire ip address print --json
roswire ip address add address=192.168.88.2/24 interface=bridge --json
roswire ip address remove .id=*1 --json
```

### 全局选项

| 选项 | 环境变量 | 说明 |
| --- | --- | --- |
| `--profile <name>` | `ROS_PROFILE` | 选择 `config.toml` 中的 profile；未设置时使用 `default_profile` |
| `--host <host>` | `ROS_HOST` | RouterOS IP 地址或 DNS 主机名；不支持 MAC 地址直连 |
| `--user <user>` | `ROS_USER` | RouterOS 用户名 |
| `--password <password>` | `ROS_PASSWORD` | RouterOS 密码；不推荐在 shell 历史记录中直接传入 |
| `--protocol <mode>` | `ROS_PROTOCOL` | `auto`、`api`、`api-ssl` 或 `rest` |
| `--routeros-version <mode>` | `ROS_ROUTEROS_VERSION` | `auto`、`v6` 或 `v7`；用于选择原生 API 方言 |
| `--transfer <mode>` | `ROS_TRANSFER` | 文件传输后端；当前唯一支持值为 `ssh` |
| `--ssh-port <port>` | `ROS_SSH_PORT` | RouterOS SSH 服务端口，默认 `22` |
| `--ssh-user <user>` | `ROS_SSH_USER` | SSH 文件传输用户名；默认复用 `ROS_USER` |
| `--ssh-password <password>` | `ROS_SSH_PASSWORD` | SSH 文件传输密码；默认复用 `ROS_PASSWORD` |
| `--ssh-key <path>` | `ROS_SSH_KEY` | SSH 私钥路径；设置后优先使用 key auth |
| `--ssh-host-key <fingerprint>` | `ROS_SSH_HOST_KEY` | RouterOS SSH host key 指纹；用于非交互校验服务器身份 |
| `--allow-from <cidr>` | `ROS_SSH_ALLOW_FROM` | 允许访问 SSH 服务的客户端来源 CIDR，用于 `/ip service ssh address` |
| `--ensure-ssh` | - | 允许 `roswire` 通过 API/REST 启用 SSH 服务并设置白名单 |
| `--restore-ssh` | - | 文件传输结束后恢复进入任务前的 SSH 服务配置 |
| `--port <port>` | `ROS_PORT` | 覆盖显式协议的默认端口；`auto` 模式下不接受单一端口覆盖 |
| `--json` | - | 输出机器可解析 JSON |
| `--debug` | `ROSWIRE_DEBUG` | 在 `stderr` 输出调试信息，必须脱敏 |

优先级：命令行参数 > 环境变量 > 本地配置 profile > 默认值。

### 本地客户端目录

`roswire` 在用户目录下维护一个本地工作目录，用于配置、日志和少量运行状态：

```text
~/.roswire/
├── config.toml        # 默认配置文件，保存 profile、非敏感选项和 secret 引用
├── logs/              # 本地 JSONL 日志，最多保留 30 天
├── state/             # 运行状态；不得保存长期会话或 RouterOS 凭据明文
└── cache/             # 可删除缓存，例如远端 schema、能力探测和短期调试材料
```

实现约束：

- 首次运行可通过 `roswire config init` 创建目录和默认 `config.toml`。
- 目录权限必须尽量收敛：Unix/macOS 目标为 `0700`，`config.toml` 目标为 `0600`。
- 如果配置文件权限过宽，且包含任何 secret 或 secret 引用，返回 `CONFIG_INSECURE_PERMISSIONS`。
- 不在 `~/.roswire/` 中保存 RouterOS 长期会话、API token、明文日志或未脱敏错误上下文。
- `ROSWIRE_HOME` 可覆盖默认目录，仅用于测试、CI 或便携环境；错误上下文中只记录脱敏后的路径尾段。

### `config.toml` 结构

配置文件支持多个 profile。默认 profile 用于未显式传入 `--profile` 的调用。

```toml
version = 1
default_profile = "home"

[profiles.home]
host = "192.168.88.1"
user = "admin"
protocol = "auto"
routeros_version = "auto"
transfer = "ssh"
ssh_port = 22
ssh_host_key = "SHA256:replace-with-routeros-host-key"
allow_from = "203.0.113.10/32"

[profiles.home.secrets.password]
type = "keychain"
service = "roswire"
account = "profiles/home/password"

[profiles.home.secrets.ssh_password]
type = "same-as"
target = "password"

[logging]
enabled = true
retention_days = 30
level = "info"
```

配置解析规则：

- `--profile <name>` 选择 profile；未设置时使用 `default_profile`。
- 命令行参数覆盖环境变量；环境变量覆盖 profile；profile 覆盖默认值。
- `config.toml` 只保存非敏感字段和 secret 引用，不默认保存明文密码。
- 所有 profile 名称、secret account、日志路径都必须做最小化校验，避免路径穿越和日志污染。

### 密码与密钥保存策略

支持三种 secret 保存方式，但安全等级不同。

| 类型 | 配置写法 | 安全等级 | 结论 |
| --- | --- | --- | --- |
| 明文 | `type = "plain"` + `value = "..."` | 低 | 仅允许实验室、一次性测试或 CI 临时目录；必须配合 `0600` 权限和显式 `allow_plain_secrets = true` |
| 本地加密 | `type = "encrypted"` + `value = "v1:..."` | 中 | 可以降低误读配置文件的风险，但不能替代钥匙链；禁止使用编译进程序的固定密钥 |
| 本机钥匙链 | `type = "keychain"` + `service/account` | 高 | 默认推荐方案；macOS Keychain、Linux Secret Service、Windows Credential Manager |

`type = "same-as"` 只是引用另一个 secret，不是第四种保存方式；解析时必须检测循环引用。

明文示例：

```toml
[profiles.lab]
host = "192.168.88.1"
user = "admin"
allow_plain_secrets = true

[profiles.lab.secrets.password]
type = "plain"
value = "lab-only-password"
```

本地加密示例：

```toml
[profiles.edge.secrets.password]
type = "encrypted"
key_id = "default"
value = "v1:base64-nonce-and-ciphertext"
```

本机钥匙链示例：

```toml
[profiles.prod.secrets.password]
type = "keychain"
service = "roswire"
account = "profiles/prod/password"
```

评估结论：

- **默认使用本机钥匙链**。配置文件只保存 `service/account` 引用，实际密码交给操作系统凭据库保存。
- **明文密码保留为显式低安全选项**。它方便实验和离线调试，但如果文件权限不安全必须拒绝运行。
- **本地加密不能使用硬编码程序密钥**。固定密钥可以从二进制中提取，本质是可逆混淆；如果实现 `encrypted`，必须使用每机生成的 master key，并优先把 master key 存入本机钥匙链。
- 如果本机钥匙链不可用，`encrypted` 模式可以后续支持 `ROSWIRE_MASTER_KEY` 这类非交互 key source；MVP 阶段不交互式询问 passphrase。
- 所有 secret 进入内存后使用专门 secret 类型承载，日志、错误、debug、dry-run、shell 建议命令都必须脱敏。
- 写入钥匙链的命令必须支持 `--stdin` 或 `--env <VAR>` 输入 secret，避免出现在 shell history 中。

推荐命令形态：

```bash
roswire config init
roswire secret set home password --stdin
roswire secret set home ssh-password --same-as password
roswire secret set lab password --plain --stdin --allow-plain
```

### 日志与保留策略

`~/.roswire/logs/` 用于本地排障日志，不能影响 `stdout` / `stderr` 契约。

- 日志格式使用 JSON Lines（JSONL），每行一个事件。
- 日志文件按日期滚动，例如 `roswire-2026-05-16.log`。
- 默认保留 30 天；MVP 阶段 30 天是上限，可以配置更短或关闭日志，不能配置更长。
- 每次启动时执行轻量清理：删除超过保留期的日志文件。清理失败只写入脱敏 warning，不阻塞主命令。
- 日志中禁止记录密码、token、SSH 私钥路径全文、本地绝对路径、RouterOS backup 文件内容或完整命令行中的敏感值。
- `--debug` 只提高本次运行的日志详细度；仍必须遵守脱敏规则。

### Agent 自描述与详细帮助

`roswire` 的主要消费者是 Agent，因此必须提供稳定、机器可读、足够详细的自描述接口。Agent 不应解析人类帮助文本来理解命令。

设计目标：

- 所有帮助、配置检查、schema 输出都支持 `--json`。
- 自描述命令默认不访问 RouterOS，不产生设备侧副作用。
- 所有输出包含 `schema_version`，便于 Agent 做兼容判断。
- 所有配置输出必须脱敏，永不输出真实密码、token、私钥内容或本地绝对敏感路径。
- 人类 `--help` 可以简洁，但 JSON help 必须详细到足以让 Agent 生成安全命令。
- `roswire help --json` 是自定义机器可读帮助子命令，不是对 clap 人类 help 文本的二次解析；`roswire --help` 保留给人类阅读。

核心命令：

| 命令 | 作用 | 是否访问 RouterOS |
| --- | --- | --- |
| `roswire help --json` | 输出完整命令目录、全局选项和帮助 schema | 否 |
| `roswire help <command...> --json` | 输出单个命令的详细参数结构、示例、输出和错误 | 否 |
| `roswire commands --json` | 输出可用命令索引，适合 Agent 快速检索 | 否 |
| `roswire commands --remote --json` | 输出本地命令目录，并叠加目标设备能力状态 | 是 |
| `roswire schema command <command...> --json` | 输出命令参数 JSON Schema | 否 |
| `roswire schema command <command...> --remote --json` | 输出命令参数 JSON Schema，并叠加远端字段、枚举值和支持状态 | 是 |
| `roswire schema output <command...> --json` | 输出成功输出 JSON Schema | 否 |
| `roswire schema discover [path...] --remote --json` | 显式连接 RouterOS，生成当前设备的能力与 schema 覆盖快照 | 是 |
| `roswire config inspect --json` | 输出当前 profile 的解析后配置、来源和脱敏 secret 状态 | 否 |
| `roswire config profiles --json` | 输出本地 profile 列表和默认 profile | 否 |
| `roswire doctor --json` | 检查本地目录、配置权限、secret 后端和依赖状态 | 默认否；传 `--include-remote` 才访问 RouterOS |
| `roswire explain-error <code> --json` | 输出错误码含义、常见原因和建议下一步 | 否 |

`config inspect` 输出必须包含字段来源，便于 Agent 判断应修改 CLI 参数、环境变量还是 profile：

```json
{
    "schema_version": "roswire.config.inspect.v1",
    "active_profile": "home",
    "paths": {
        "home": "~/.roswire",
        "config": "~/.roswire/config.toml",
        "logs": "~/.roswire/logs"
    },
    "resolved": {
        "host": { "value": "192.168.88.1", "source": "profile" },
        "user": { "value": "admin", "source": "profile" },
        "protocol": { "value": "auto", "source": "default" },
        "transfer": { "value": "ssh", "source": "profile" },
        "password": { "status": "available", "type": "keychain", "source": "profile", "redacted": true },
        "ssh_host_key": { "value": "SHA256:...", "source": "profile" }
    },
    "logging": {
        "enabled": true,
        "retention_days": 30,
        "level": "info"
    },
    "warnings": []
}
```

命令帮助 JSON 必须包含以下结构：

```json
{
    "schema_version": "roswire.command.help.v1",
    "name": "ip address add",
    "summary": "添加 RouterOS IP address 记录。",
    "stability": "planned",
    "kind": "routeros-command",
    "syntax": "roswire ip address add address=<cidr> interface=<name> [disabled=<bool>] --json",
    "routeros_path": "/ip/address/add",
    "protocol_support": {
        "classic_api_v6": true,
        "classic_api_v7": true,
        "rest_v7": true,
        "rest_method": "PUT",
        "rest_path": "/rest/ip/address"
    },
    "side_effects": ["creates-routeros-record"],
    "idempotency": "not-idempotent",
    "requires_confirmation": false,
    "arguments": [
        {
            "name": "address",
            "style": "key-value",
            "required": true,
            "type": "cidr",
            "example": "192.168.88.2/24",
            "description": "要添加到接口上的 IP 地址。"
        },
        {
            "name": "interface",
            "style": "key-value",
            "required": true,
            "type": "string",
            "example": "bridge",
            "discovery_hint": "可先运行 roswire interface print --json 获取可用接口。"
        }
    ],
    "examples": [
        {
            "title": "给 bridge 添加地址",
            "command": "roswire ip address add address=192.168.88.2/24 interface=bridge --json",
            "expected_stdout_shape": "object",
            "notes": ["失败时检查 interface 是否存在。"]
        }
    ],
    "success_output": {
        "content_type": "application/json",
        "schema_ref": "roswire.output.routeros-record.v1"
    },
    "errors": ["USAGE_ERROR", "AUTH_FAILED", "NETWORK_ERROR", "ROS_API_FAILURE"],
    "self_correction": [
        {
            "when_error_contains": "no such interface",
            "next_command": "roswire interface print --json"
        }
    ]
}
```

命令目录规范：

- 每个命令必须登记 `kind`：`routeros-command`、`workflow`、`config`、`secret`、`introspection`。
- 每个会修改设备状态的命令必须声明 `side_effects` 和 `idempotency`。
- 每个参数必须声明 `style`：`positional`、`key-value`、`flag`、`option`。
- 每个参数必须声明 `required`、`type`、`description`，并尽量提供 `example`。
- 文件工作流必须声明本地路径、远端路径、临时文件、清理策略和 SSH 前置条件。
- 帮助示例中的命令不得包含真实密码、token、私钥路径或公网真实敏感地址。
- `doctor --include-remote --json` 才允许访问 RouterOS；普通 `help/schema/config inspect` 必须纯本地。

### 动态能力与 schema 发现

RouterOS 的菜单、字段和命令会随版本、package、硬件能力和授权状态变化。把所有 schema 都硬编码进 `roswire` 会很快过期；但反过来，完全依赖设备动态返回也不可靠，因为 RouterOS 官方公开文档中没有承诺提供完整、稳定、可直接生成 OpenAPI/JSON Schema 的元数据端点。REST API 本质上是 console API 的 JSON wrapper，支持通过 `POST` 调用任意 console command，但不等于提供完整 schema 注册表。

设计结论：采用“静态目录 + 远端覆盖 + 缓存”的混合模型。

| 层级 | 来源 | 用途 | 限制 |
| --- | --- | --- | --- |
| 静态目录 | `roswire` 随版本内置的命令目录、参数说明、示例、错误码和安全策略 | Agent 离线理解命令、生成基础调用、做稳定 snapshot 测试 | 需要随 `roswire` 发布更新，可能落后于最新 RouterOS |
| 远端覆盖 | 显式连接设备后，通过只读探测、版本信息、package 信息、协议能力和可观测字段生成 | 判断当前设备是否支持某命令、字段、协议和枚举候选值 | 只能作为 capability overlay，不应覆盖安全策略和副作用声明 |
| 运行时值发现 | 通过 `interface print`、`routing table print`、`system resource print` 等只读命令获取 | 为 Agent 提供接口名、表名、bridge 名称等候选值 | 这是当前设备数据，不是通用 schema；必须标记来源和时间 |
| 本地缓存 | `~/.roswire/cache/schema/` 下的可删除 JSON 快照 | 减少重复探测成本，便于 Agent 快速启动 | 缓存可失效、可删除、不可含 secret；不能作为唯一真相 |

动态发现命令必须显式要求访问远端：

```bash
roswire schema discover --remote --json
roswire schema discover ip address --remote --json
roswire schema command ip address add --remote --json
roswire commands --remote --json
```

默认的 `help --json`、`commands --json`、`schema command ... --json` 仍然只读取本地静态目录，不访问 RouterOS。这样可以保持自描述命令无副作用、低延迟、可离线，也避免 Agent 在“只是想查帮助”时误触发网络访问。

远端发现流程：

1. 按正常协议路由规则连接设备，解析 RouterOS 主版本、完整版本、build time、architecture、board-name 和 package 列表摘要。
1. 记录当前命中的协议：`rest`、`api-ssl` 或 `api`，以及原生 API 方言：`v6` 或 `v7`。
1. 对目标路径执行只读 capability probe，例如 `print`、`print count-only`、有限 `.proplist`、REST `GET` 或 REST `POST <path>/print`。
1. 从成功响应中提取可观测输出字段；从失败响应中只提取脱敏错误分类，不能把失败消息当成可靠 schema。
1. 对枚举值只做“候选值”标注，例如接口名、routing table、bridge、address-list 名称；不能把当前候选值误判为完整枚举。
1. 生成 `remote_overlay`，再与静态目录合并输出。
1. 如果启用缓存，把结果写入 `~/.roswire/cache/schema/<cache-key>.json`。

远端发现必须遵守安全边界：

- 只允许执行只读命令；禁止为了发现参数而执行 `add`、`set`、`remove`、`enable`、`disable`、`reset`、`run`、`import`、`backup save` 等有副作用命令。
- 禁止通过“故意发错参数再解析错误消息”的方式构造必填字段 schema。错误文本不稳定，而且容易制造设备日志噪音。
- `remote_overlay` 不能改变静态目录中的 `side_effects`、`idempotency`、secret 标记、文件传输安全规则和危险命令策略。
- 如果设备账号权限不足，返回降级后的 schema，并在 `warnings` 中标记 `CAPABILITY_PROBE_FAILED`；不能要求 Agent 提权。
- 动态发现默认不修改 SSH、API、REST 或防火墙服务配置；如果发现命令需要 SSH 文件传输，只能报告前置条件缺失。

缓存键建议包含：

```text
profile-name + host-id + routeros-version + build-time + architecture + board-name + packages-hash + selected-protocol
```

其中 `host-id` 必须是脱敏哈希，不能直接写入真实主机名、公网 IP 或用户名。缓存失效规则：

- RouterOS 版本、build time、package 摘要、协议能力任一变化时失效。
- 默认 TTL 不超过 7 天；用户可以配置更短，也可以关闭动态 schema 缓存。
- `roswire schema discover --remote --refresh --json` 强制绕过缓存重新探测。
- 普通命令执行不能因为缓存缺失而自动做全量 schema 发现；最多复用首次连接阶段已经拿到的版本与协议能力。

远端 schema 输出示例：

```json
{
    "schema_version": "roswire.remote.schema.v1",
    "schema_source": ["static_catalog", "remote_overlay", "runtime_values"],
    "profile": "home",
    "device": {
        "routeros_version": "7.15.3",
        "major": "v7",
        "architecture": "arm64",
        "board_name": "RB5009UG+S+",
        "selected_protocol": "rest"
    },
    "cache": {
        "status": "miss",
        "ttl_seconds": 604800,
        "cache_key": "sha256:..."
    },
    "commands": [
        {
            "name": "ip address add",
            "support": "supported",
            "schema_source": ["static_catalog", "remote_overlay"],
            "output_fields_observed": [".id", "address", "interface", "network", "disabled"],
            "runtime_value_hints": {
                "interface": ["bridge", "ether1"]
            },
            "warnings": []
        }
    ],
    "warnings": []
}
```

Agent 使用建议：

- 生成命令前先读本地 `commands --json` 或 `help <command> --json`。
- 需要匹配真实设备版本、接口名、REST/API 支持状态时，再调用 `schema ... --remote --json`。
- 如果 `remote_overlay` 与静态目录冲突，安全策略以静态目录为准；字段存在性和协议支持以远端覆盖为准。
- 如果远端探测失败，Agent 应根据 `warnings` 降级使用静态目录，而不是猜测未知字段。

### 默认端口

| 协议 | 默认端口 | 说明 |
| --- | ---: | --- |
| `api` | `8728` | RouterOS 原生 API |
| `api-ssl` | `8729` | 基于 TLS 的 RouterOS 原生 API |
| `rest` | `443` | RouterOS v7 REST，实际端口取决于设备上的 `www-ssl` 服务配置 |

### 协议自动选择与调用优先级

`ROS_PROTOCOL=auto` 是默认模式。它用于解决 RouterOS v7 同时支持原生 API 与 REST API 时的路由选择问题。

基本原则：显式配置优先，自动探测其次。

| 场景 | 选择策略 |
| --- | --- |
| 用户显式指定 `--protocol rest` | 只走 REST；如果设备不是 v7 或 REST 不可用，返回结构化错误 |
| 用户显式指定 `--protocol api` 或 `api-ssl` | 只走原生 API，并按 `--routeros-version` 选择或探测 v6/v7 方言 |
| `auto` + RouterOS v6 | 走原生 API v6 方言 |
| `auto` + RouterOS v7 + REST 可用 + 当前动作有 REST 映射 | 优先走 REST |
| `auto` + RouterOS v7 + REST 不可用 | 回落到原生 API v7 方言 |
| `auto` + RouterOS v7 + 当前动作没有 REST 映射 | 回落到原生 API v7 方言 |

自动探测候选顺序：

1. `rest`：尝试只读请求 `/rest/system/resource`，解析版本与 REST 可用性。
1. `api-ssl`：登录后执行 `/system/resource/print`，解析版本。
1. `api`：登录后执行 `/system/resource/print`，解析版本。

错误处理规则：

- 网络不可达、端口未开放、服务未启用：记录为候选失败，继续尝试下一个协议。
- 认证失败：立即返回 `AUTH_FAILED`，不再静默尝试其它协议，避免掩盖凭据问题。
- REST 返回 404/405 或缺少目标菜单映射：视为该动作不适合 REST，允许回落到 v7 原生 API。
- `auto` 模式下如果用户同时指定 `--port`，返回 `CONFIG_ERROR`；避免一个端口被误用于多个候选协议。

首次探测结果只在当前进程内复用，不写入磁盘缓存。CLI 仍保持无状态；下一次调用会重新探测，除非用户显式指定协议和版本。

### 原生 API 方言选择

RouterOS v6 与 v7 的原生 API 共享同一套 TCP/TLS sentence 协议，但不能在实现中完全混为一个后端。差异主要体现在登录流程、菜单字段、命令可用性、`!trap` 错误内容和返回字段归一化上。

设计结论：采用“共享传输层 + 分离方言层”。

- 共享部分：TCP/TLS 连接、word/sentence 编解码、`!re`/`!done`/`!trap` 解析、超时处理、脱敏日志、统一错误输出。
- v6 方言层：兼容旧式 challenge-response 登录与 v6.43+ 的现代登录流程，维护 v6 字段归一化和命令差异。
- v7 方言层：默认使用现代登录流程，维护 v7 字段归一化、命令差异，并与 REST 映射保持语义一致。
- `auto` 模式：默认先尝试现代登录；如遇到旧式登录特征再回退到 v6 challenge-response。登录成功后通过 `/system/resource/print` 的版本信息确认方言。
- 显式模式：`--routeros-version v6` 或 `--routeros-version v7` 跳过模糊探测，适合测试、CI 和受控生产环境。

允许 v6/v7 方言模块在映射表、归一化逻辑、错误 hint 上保留适度冗余。禁止复制底层 sentence codec、TCP/TLS I/O 和统一错误输出，否则后续 bug 修复会变成双份维护。

## 4. 数据模型与协议映射

Agent 通常使用 CLI 风格的层级命令。`roswire` 需要将空格分隔路径转换为 RouterOS API 路径，并将 `key=value` 参数转换为协议载荷。

### 路径映射

| CLI | 原生 API（v6/v7 方言） | REST API |
| --- | --- | --- |
| `roswire ip address print` | `/ip/address/print` | `GET /rest/ip/address` |
| `roswire ip address add address=1.1.1.1 interface=ether1` | `/ip/address/add` + `=address=1.1.1.1` + `=interface=ether1` | `PUT /rest/ip/address` + JSON 请求体 |
| `roswire ip address set .id=*1 disabled=true` | `/ip/address/set` + `=.id=*1` + `=disabled=true` | `PATCH /rest/ip/address/*1` + JSON 请求体 |
| `roswire ip address remove .id=*1` | `/ip/address/remove` + `=.id=*1` | `DELETE /rest/ip/address/*1` |

> REST 动词映射必须在实现阶段用 RouterOS v7 文档与真实设备测试验证；不要仅根据常见 REST 习惯推断。

### 参数规则

- `key=value` 原样保留 RouterOS 字段名，例如 `.id=*1`、`comment=wan uplink`。
- 布尔值、数字和字符串在 CLI 层保持字符串；协议层按目标 API 要求转换。
- 任何密码、令牌（token）、证书内容都不能出现在错误上下文或调试日志中。
- 未知动作（action）不应静默猜测；返回 `UNSUPPORTED_ACTION`，或在后续版本中设计显式原始命令（raw command）透传模式。

### 客户端文件上传下载

这里讨论的是本机运行 `roswire` 时，如何把本机文件上传到 RouterOS，或把 RouterOS 上的文件下载到本机。它不同于 `/file print`、`/file remove` 这类设备内文件菜单操作。

设计结论：`roswire` 应该提供一等公民的文件工作流，但内部必须分成“API/REST 编排”和“SSH 字节传输”两层。

- API/REST 负责控制面：生成备份、导出配置、执行导入、查询文件是否存在、删除临时文件。
- SSH transfer 后端负责数据面：真正把文件字节从本机传到路由器，或从路由器传回本机。
- 不能把 API sentence 或 REST JSON 当成通用大文件传输通道。

目标命令：

| 命令 | 工作流 |
| --- | --- |
| `roswire file upload <local> <remote>` | 通过 SSH transfer 后端上传本机文件到 RouterOS 文件系统 |
| `roswire file download <remote> <local>` | 通过 SSH transfer 后端把 RouterOS 文件下载到本机 |
| `roswire import <local.rsc>` | 上传 `.rsc` 到临时路径，执行 `/import file-name=...`，可选清理远端临时文件 |
| `roswire export download <local.rsc>` | 执行 `/export file=...`，等待文件出现，下载到本机，可选清理远端文件 |
| `roswire backup download <local.backup>` | 执行 `/system/backup/save name=...`，等待 `.backup` 文件出现，下载到本机，可选清理远端文件 |
| `roswire script put <name> --source @<local.rsc>` | 读取本地文本，通过 API/REST 写入 `/system/script source=...`，不创建 RouterOS 文件 |

SSH 认证规则：

- 默认复用 `ROS_USER` / `ROS_PASSWORD` 作为 SSH 登录凭据，减少重复配置。
- 如果提供 `--ssh-user`、`--ssh-password` 或 `--ssh-key`，则 SSH transfer 使用显式 SSH 凭据，不影响 API/REST 控制面登录。
- 设置 `--ssh-key` 时优先使用 key auth；私钥 passphrase 不得交互式询问，必须通过后续安全机制设计，MVP 阶段可先不支持加密私钥。
- SSH host key 必须非交互校验。MVP 阶段要求用户通过 `--ssh-host-key` 或 `ROS_SSH_HOST_KEY` 提供期望指纹；未知或不匹配时失败。
- 所有 SSH 凭据都属于敏感信息，不能进入错误上下文、debug 日志、dry-run 输出或 shell 建议命令。

SSH 服务准备：

| 步骤 | API/REST 动作 | 说明 |
| --- | --- | --- |
| 检查 SSH 服务 | `/ip/service/print where name=ssh` | 读取 `disabled`、`port`、`address` 当前值 |
| 启用 SSH | `/ip/service/set ssh disabled=no` | 仅当用户显式传入 `--ensure-ssh` 时允许执行 |
| 设置白名单 | `/ip/service/set ssh address=<merged-cidrs>` | 只允许把 `<cidr>` 追加/合并进现有 `address` 列表，禁止无提示替换为更宽范围 |
| 设置端口 | `/ip/service/set ssh port=<port>` | 仅当用户显式传入 `--ssh-port` 或 `ROS_SSH_PORT` 时修改 |
| 恢复配置 | `/ip/service/set ssh ...` | 仅当用户传入 `--restore-ssh` 时恢复任务前快照 |

SSH 白名单安全规则：

- `--ensure-ssh` 需要修改 SSH 服务时，必须提供 `--allow-from` 或 `ROS_SSH_ALLOW_FROM`；不能默认写入 `0.0.0.0/0` 或 `::/0`。
- 如果当前 `/ip service ssh address` 已经覆盖客户端来源，则不需要修改白名单。
- 如果需要修改白名单，只能基于任务前快照做追加/合并，不能覆盖掉管理员已有地址限制。
- 如果传入的 CIDR 明显过宽（例如 `0.0.0.0/0`、`::/0`），返回 `SSH_WHITELIST_UNSAFE`。
- 如果启用了 `--restore-ssh`，必须在成功、失败和中断路径中尽力恢复任务前 SSH 服务配置；恢复失败要写入 `stderr` 结构化错误或警告，不能静默吞掉。

不做默认承诺：

- 不默认假设 REST 支持 multipart/form-data 上传。
- 不保留 FTP、`/tool fetch`、临时 HTTP server 等其它传输后端。
- 不在未传入 `--ensure-ssh` 时自动开启 RouterOS SSH 服务。
- 不自动修改防火墙规则；本阶段只管理 `/ip service ssh address` 白名单。
- 不默认通过 API/REST 读取或写入大文件内容。RouterOS 文件菜单对 `contents` 和 `/file/read` 有明确大小/分块限制，只能作为小文本或诊断能力。
- 不在用户未请求文件工作流时探测或修改 SSH 服务；普通 RouterOS 命令路径不应触碰 SSH。

MVP 阶段可以先实现 `dry-run` 计划和清晰错误：如果 SSH 服务未启用且用户没有传入 `--ensure-ssh`，返回 `SSH_SERVICE_UNAVAILABLE`；如果缺少白名单，返回 `SSH_WHITELIST_REQUIRED`。

所有文件传输命令都必须有：超时、大小限制、校验和校验、覆盖策略、临时文件清理策略和脱敏日志。文件路径必须做最小化规范化，禁止把本地绝对路径或敏感目录写入错误 payload。

## 5. 标准错误模型

为了让 Agent 能够解析错误并进行自我修正，所有错误必须使用稳定的 JSON 结构。

```rust
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct RosWireError {
    pub error_code: String,
    pub message: String,
    pub hint: Option<String>,
    pub context: ErrorContext,
}

#[derive(Serialize)]
pub struct ErrorContext {
    pub command: String,
    pub requested_protocol: String,
    pub selected_protocol: String,
    pub transfer_backend: Option<String>,
    pub routeros_version: String,
    pub host: String,
    pub resolved_args: BTreeMap<String, String>,
}
```

`requested_protocol` 记录用户请求值，例如 `auto`、`rest`、`api-ssl`。`selected_protocol` 记录实际命中的协议，例如 `rest`、`api-ssl`、`api` 或 `unknown`。`transfer_backend` 仅在文件传输工作流中设置，例如 `ssh`。`routeros_version` 使用解析后的方言值，例如 `v6`、`v7` 或 `unknown`。在登录失败、版本探测失败等早期错误路径中，不能为了填充这些字段而发起额外有副作用的探测命令。

### 错误码建议

| 错误码 | 含义 | 退出码 |
| --- | --- | ---: |
| `USAGE_ERROR` | CLI 参数缺失或格式错误 | `2` |
| `CONFIG_ERROR` | 环境变量或配置不完整 | `2` |
| `PROFILE_NOT_FOUND` | 请求的本地配置 profile 不存在 | `2` |
| `CONFIG_INSECURE_PERMISSIONS` | `~/.roswire` 或 `config.toml` 权限过宽 | `2` |
| `SECRET_BACKEND_UNAVAILABLE` | 请求的 secret 后端不可用，例如系统钥匙链不可用 | `2` |
| `SECRET_NOT_FOUND` | 配置引用的 secret 不存在 | `2` |
| `SECRET_DECRYPT_FAILED` | 本地加密 secret 解密失败 | `3` |
| `AUTH_FAILED` | RouterOS 认证失败 | `3` |
| `NETWORK_ERROR` | TCP/TLS/HTTP 连接失败 | `4` |
| `ROS_API_FAILURE` | RouterOS 返回 trap/error | `1` |
| `UNSUPPORTED_ACTION` | 当前 action 无映射或未实现 | `2` |
| `SSH_SERVICE_UNAVAILABLE` | RouterOS SSH 服务未启用，且用户未允许自动开启 | `2` |
| `SSH_HOST_KEY_REQUIRED` | SSH 文件传输缺少 RouterOS host key 指纹 | `2` |
| `SSH_HOST_KEY_MISMATCH` | RouterOS SSH host key 与期望指纹不匹配 | `3` |
| `SSH_WHITELIST_REQUIRED` | SSH 文件传输缺少来源白名单 CIDR | `2` |
| `SSH_WHITELIST_UNSAFE` | SSH 白名单 CIDR 过宽或会放宽现有访问限制 | `2` |
| `SSH_RESTORE_FAILED` | 文件传输后恢复 SSH 服务配置失败 | `6` |
| `FILE_TOO_LARGE` | 文件超过当前后端或安全策略限制 | `2` |
| `FILE_TRANSFER_FAILED` | SSH 文件传输执行失败 | `6` |
| `SERIALIZATION_ERROR` | JSON 编解码失败 | `5` |
| `HELP_TOPIC_NOT_FOUND` | 请求的帮助主题或命令不存在 | `2` |
| `SCHEMA_UNAVAILABLE` | 请求的命令 schema 尚未登记 | `2` |
| `REMOTE_SCHEMA_UNAVAILABLE` | 远端 schema 或能力覆盖不可用，只能使用静态目录 | `2` |
| `CAPABILITY_PROBE_FAILED` | 远端只读能力探测失败，输出已降级 | `1` |
| `REMOTE_SCHEMA_STALE` | 本地远端 schema 缓存已过期或与设备指纹不匹配 | `2` |
| `INTERNAL_ERROR` | 未预期内部错误 | `5` |

## 6. 模块划分

```text
src/
├── main.rs            # 入口；全局错误捕获；stdout/stderr 分流
├── args.rs            # clap 定义与 CLI 解析
├── error.rs           # 统一错误类型与 JSON 输出
├── mapping.rs         # CLI path/action 到协议请求的转换
├── config/
│   ├── mod.rs         # 配置解析入口与优先级合并
│   ├── paths.rs       # ~/.roswire、ROSWIRE_HOME、权限检查
│   ├── file.rs        # config.toml 读写与 schema version
│   ├── profiles.rs    # profile 解析、继承与默认值
│   ├── secrets.rs     # plain/encrypted/keychain secret 解析与脱敏
│   └── logging.rs     # JSONL 日志、滚动与 30 天保留策略
├── introspect/
│   ├── mod.rs         # Agent 自描述入口
│   ├── catalog.rs     # 命令目录与索引
│   ├── help.rs        # 机器可读详细帮助
│   ├── schema.rs      # 参数与输出 JSON Schema
│   ├── discovery.rs   # 远端 schema/capability 只读探测与覆盖合并
│   ├── cache.rs       # ~/.roswire/cache/schema 下的动态 schema 缓存
│   ├── config.rs      # config inspect/profiles 的脱敏输出
│   └── doctor.rs      # 本地/远端诊断检查
├── workflow/          # 跨协议编排，不直接实现底层协议
│   ├── mod.rs         # 工作流入口与计划模型
│   ├── import.rs      # 上传 .rsc 并执行 /import
│   ├── export.rs      # 生成并下载 .rsc
│   ├── backup.rs      # 生成并下载 .backup
│   └── script.rs      # 小文本写入 /system/script source
├── protocol/
│   ├── mod.rs              # Protocol trait 与协议路由器
│   ├── discovery.rs        # 首次连接探测、协议优先级与能力选择
│   ├── classic/
│   │   ├── mod.rs          # 原生 API 共享入口
│   │   ├── transport.rs    # TCP/TLS 连接与超时控制
│   │   ├── sentence.rs     # word/sentence 编解码
│   │   ├── login.rs        # 现代登录与旧式 v6 登录回退
│   │   ├── dialect.rs      # v6/v7 方言 trait 与能力描述
│   │   ├── v6.rs           # RouterOS v6 方言实现
│   │   └── v7.rs           # RouterOS v7 方言实现
│   └── rest_v7.rs          # RouterOS v7 REST 实现
└── transfer/               # 客户端 ↔ RouterOS SSH 文件传输
    ├── mod.rs              # 文件传输 trait、计划模型与统一错误映射
    ├── ssh.rs              # SSH 文件上传下载实现
    └── ssh_service.rs      # 通过 API/REST 检查、启用、白名单与恢复 SSH 服务
```

## 7. 依赖策略

限制依赖是项目目标之一，但“零运行时依赖”不等于“零 Rust crate”。优先选择同步、阻塞式实现，避免为了单次 CLI 调用引入完整异步运行时。

推荐起点：

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
toml = "0.8"
directories = "6"
keyring = "3"
secrecy = "0.10"
zeroize = "1"
chacha20poly1305 = "0.10"
rand = "0.9"
schemars = "0.8"

# REST 客户端候选：实现时二选一，并禁用默认 TLS 后端。
# reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
# ureq = { version = "3", default-features = false, features = ["json", "rustls"] }

# SSH 文件传输：选用 ssh2，统一封装 SCP/SFTP 能力。
ssh2 = "0.9"
```

约束：

- 不默认使用 `tokio = { features = ["full"] }`，除非后续证明需要长连接或并发任务。
- TLS 优先使用 Rustls，减少系统 OpenSSL 依赖带来的发布复杂度。
- SSH 文件传输依赖固定为 `ssh2`。实现时优先尝试 SFTP；如果目标 RouterOS 版本不支持 SFTP，再验证是否可用 SCP。
- `ssh2` 会引入 `libssh2` 及其 native 依赖链，发布计划必须覆盖 macOS/Linux 静态或半静态打包。
- 如果 `ssh2` 在目标平台无法可靠打包，必须先记录为发布 blocker，不自动切换到其它 SSH 库。
- `keyring` 是默认 secret 后端依赖；如果目标平台没有可用钥匙链，必须返回结构化错误，不回退为明文。
- `chacha20poly1305` 只用于 `encrypted` secret；master key 不能硬编码进程序。
- `schemars` 用于生成和测试 Agent 可读 JSON Schema；命令 help 仍需要维护人工说明、示例和自愈提示。
- 所有新增依赖都必须说明用途、二进制体积影响与替代方案。

## 8. 测试计划

### 单元测试

- CLI path/action 解析与协议映射。
- `stdout`/`stderr` 分流，不允许错误进入 `stdout`。
- 错误 JSON 快照，字段顺序和脱敏规则必须稳定。
- 配置优先级：CLI 参数 > 环境变量 > 本地配置 profile > 默认值。
- 本地配置目录：`ROSWIRE_HOME` 覆盖、`~/.roswire` 创建、权限过宽返回 `CONFIG_INSECURE_PERMISSIONS`。
- `config.toml`：profile 选择、schema version、缺失字段默认值和未知字段报错策略。
- profile 选择：`--profile`、`ROS_PROFILE`、`default_profile` 优先级和 profile 不存在错误。
- Secret 解析：plain 需要显式允许、encrypted 解密失败、keychain 缺失、same-as 引用循环。
- 日志保留：默认 30 天、不能配置更长、清理过期文件、日志脱敏。
- Agent 自描述：`help --json`、`help <command> --json`、`commands --json`、`schema command/output --json` 输出稳定 snapshot。
- 动态 schema：默认 schema 命令不访问远端；`--remote` 才探测 RouterOS；缓存键、TTL、失效和脱敏规则稳定。
- 远端覆盖合并：静态目录的安全策略不被覆盖，远端字段和协议支持可以修正 capability。
- 能力探测降级：权限不足、路径不存在、REST/API 差异返回 `CAPABILITY_PROBE_FAILED` warning，而不是崩溃或猜测 schema。
- 配置检查：`config inspect --json` 输出来源、脱敏 secret 状态、日志策略和 warnings。
- 帮助安全：帮助示例、doctor 输出和 dry-run 不包含真实 secret 或本地敏感绝对路径。
- `auto` 协议选择：v7 同时可用 REST 与原生 API 时 REST 优先。
- `auto` 回落：REST 不可用或动作无 REST 映射时回落到 v7 原生 API。
- 显式协议优先：`--protocol api`、`api-ssl`、`rest` 不被自动改道。
- 认证失败不回落：任一候选协议确认认证失败后立即返回 `AUTH_FAILED`。
- SSH 文件传输工作流：上传脚本、导入 `.rsc`、生成并下载 backup/export 的计划生成。
- SSH 服务准备：未启用 SSH 且缺少 `--ensure-ssh` 时返回 `SSH_SERVICE_UNAVAILABLE`。
- SSH host key：缺少 `--ssh-host-key` / `ROS_SSH_HOST_KEY` 时返回 `SSH_HOST_KEY_REQUIRED`；不匹配时返回 `SSH_HOST_KEY_MISMATCH`。
- SSH 白名单：需要修改白名单但缺少 `--allow-from` 或 `ROS_SSH_ALLOW_FROM` 时返回 `SSH_WHITELIST_REQUIRED`。
- SSH 白名单安全：拒绝 `0.0.0.0/0`、`::/0`，且不会覆盖管理员已有地址限制。
- SSH 服务恢复：成功、失败和中断路径都生成恢复计划；恢复失败返回 `SSH_RESTORE_FAILED`。

### 集成测试

- RouterOS CHR 或测试设备上的原生 API 登录、print、add、set、remove。
- RouterOS v6 原生 API：旧式 challenge-response 登录、v6.43+ 现代登录、典型字段差异。
- RouterOS v7 原生 API：现代登录、典型字段差异、与 REST 语义一致性。
- RouterOS v7 REST 登录、print、add、set、remove。
- RouterOS v7 双协议环境：验证 REST 优先、原生 API 回落和错误上下文中的实际协议。
- 认证失败、接口不存在、TLS 失败、REST 404/401 等错误路径。
- SSH 文件传输后端：验证上传 `.rsc` 后导入、生成 `.backup` 后下载、白名单设置、服务恢复、大小限制、超时、覆盖策略和校验和失败路径。
- SSH 凭据路径：密码复用、显式 SSH 凭据、key auth、host key 校验、加密私钥不支持路径都要有稳定错误。
- 本机钥匙链：macOS Keychain、Linux Secret Service、Windows Credential Manager 至少各有 smoke test 或 documented fallback。
- Agent 远端诊断：`doctor --include-remote --json` 访问 RouterOS 时必须声明 selected protocol、routeros version、服务状态和脱敏错误。

### 发布验证

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- Linux/macOS 发布二进制冒烟测试。

## 9. 里程碑

| 阶段 | 主题 | 交付物 |
| --- | --- | --- |
| M0 | 项目初始化 | 创建 `Cargo.toml`、`src/main.rs`、基础 CI |
| M1 | CLI、本地配置与自描述 | 实现 `args.rs`、`config/`、`introspect/`、`error.rs`、`mapping.rs`、`~/.roswire/config.toml` 和静态 schema 目录 |
| M2 | 协议抽象与自动选择 | 定义 `Protocol` trait、统一请求/响应类型、首次探测与优先级策略 |
| M3 | 原生 API 共享层 | 实现 RouterOS API sentence（协议句）编解码、TCP/TLS 传输和登录流程 |
| M4 | v6/v7 方言 | 实现原生 API v6/v7 方言、字段归一化和兼容测试 |
| M5 | REST API | 实现 v7 REST 映射、HTTP 状态码归一化和 JSON 解析 |
| M6 | 动态 schema 与 SSH 文件传输 | 实现远端 schema 覆盖、缓存、SSH 服务准备、白名单合并/恢复、上传下载和文件工作流编排 |
| M7 | 测试与文档 | 完成单元测试、集成测试、README 示例校正 |
| M8 | 发布 | GitHub Releases、校验和、基础安装说明 |

## 10. Agent 自愈闭环

当 Agent 调用 `roswire` 失败时，设计闭环如下：

1. Agent 执行 `roswire ip address add interface=ether99 address=10.0.0.1/24 --json`。
1. RouterOS 返回 `no such interface (ether99)`。
1. `roswire` 向 `stderr` 写入标准 JSON 错误，退出码为 `1`。
1. Agent 运行时检测到非零退出码，读取并解析 `stderr`。
1. Agent 从 `message` 或 `hint` 中识别接口不存在。
1. Agent 自动执行 `roswire interface print --json` 获取可用接口，再生成下一步修正命令。

## 11. 给实现 Agent 的启动提示

```text
你是一名资深 Rust 工程师。请按照 docs/develop-plan.md 实现 roswire。

第一项任务：
1. 初始化 Rust 项目，创建 Cargo.toml 与 src/main.rs。
2. 实现 src/error.rs、src/args.rs、src/config/ 与 src/mapping.rs。
3. 实现 ROS_PROTOCOL=auto 的协议探测骨架：REST、api-ssl、api 的候选顺序和错误分类。
4. 实现 SSH 文件传输的 CLI 配置骨架：--transfer ssh、--ssh-host-key、--ensure-ssh、--allow-from、--restore-ssh。
5. 实现 Agent 自描述骨架：help --json、commands --json、config inspect --json、schema command --json；默认只读本地静态目录。
6. 预留动态 schema 骨架：schema discover --remote --json、schema command ... --remote --json、远端覆盖合并与 ~/.roswire/cache/schema 缓存接口。
7. 确保错误只以稳定 JSON 写入 stderr。
8. 默认载荷中不要包含时间戳或随机 ID。
9. 使用 BTreeMap 保证参数排序稳定。
10. 为 CLI 解析、本地配置、secret 解析、日志保留、Agent 自描述、动态 schema 缓存、协议自动选择、SSH 服务准备计划与错误序列化添加单元测试。
```
