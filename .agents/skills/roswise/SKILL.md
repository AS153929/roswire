---
name: roswire
description: 当需要使用本机 `roswire` 命令行管理 RouterOS/MikroTik 设备时使用，包括查看已配置设备、增加或更新设备、查看设备配置、检查外网联通状态和网络位置、查看路由表、接口、防火墙、WireGuard、Netwatch、软件包、用户、命令 schema 发现，以及安全执行只读 raw RouterOS print 命令。
---

# Roswire

## 基本规则

使用本机 `roswire` 命令通过已配置的 profile 管理 RouterOS 设备。默认使用 JSON 输出，方便解析、筛选和总结：

```bash
roswire --json <command tokens...>
roswire --json --profile <profile> <command tokens...>
```

除非 schema 明确显示某个 flag 属于具体命令，否则把全局选项放在命令 token 之前，例如 `--json`、`--profile`、`--host`、`--user`、`--protocol`、`--routeros-version`、`--dry-run`、`--remote`、`--refresh`。

采用这些安全默认值：

- 检查、查询类任务默认加 `--json`，除非用户明确要人类可读文本。
- 计划修改配置前先用 `--dry-run`。
- 不要把密码直接写进 shell 历史或最终回复；设置密钥优先使用 `--stdin`。
- 除非用户明确要求写操作，并且风险已经说明清楚，否则不要使用 `--allow-write`。
- 默认把 `raw` 当作只读能力使用：只执行以 `/print` 结尾的 RouterOS 路径。
- 不确定命令形状时，先运行 `roswire --json commands`，再运行 `roswire --json schema command <topic...>`。

## 命令发现

查看 roswire 内置命令索引：

```bash
roswire --json commands
roswire help
```

查看某个命令主题的参数 schema：

```bash
roswire --json schema command config device add
roswire --json schema command ip route print
roswire --json schema command raw
```

当已有真实设备 profile 时，可发现远端 schema overlay：

```bash
roswire --json --profile <profile> --remote schema discover
roswire --json --profile <profile> --remote --refresh schema discover
```

## 设备 Profile

查看已配置的设备/profile 以及默认 profile：

```bash
roswire --json config profiles
```

如果 roswire 本地 home/config 不存在，先初始化：

```bash
roswire --json config init
```

增加新的设备 profile：

```bash
roswire --dry-run --json config device add <profile> host=<host-or-ip> user=<username> protocol=auto routeros_version=auto transfer=ssh
roswire --json config device add <profile> host=<host-or-ip> user=<username> protocol=auto routeros_version=auto transfer=ssh
```

从 help 信息可用的连接选项：

- `protocol=auto|api|api-ssl|rest`
- `routeros_version=auto|v6|v7`
- `transfer=ssh`

通过 stdin 设置 profile 密码或其他 secret。优先使用 `type=keychain`；只有在环境合适时才使用 `plain` 或 `encrypted`。

```bash
read -rs ROUTER_PASSWORD
printf '%s' "$ROUTER_PASSWORD" | roswire --stdin --json config secret set <profile> password type=keychain
unset ROUTER_PASSWORD
```

更新已有设备 profile：

```bash
roswire --dry-run --json config device set <profile> host=<new-host> user=<username> protocol=auto routeros_version=auto transfer=ssh
roswire --json config device set <profile> host=<new-host> user=<username> protocol=auto routeros_version=auto transfer=ssh
```

查看默认 profile 或指定 profile 的最终解析配置。该命令会展示配置来源优先级，并会隐藏 secret 值：

```bash
roswire --json config inspect
roswire --json --profile <profile> config inspect
```

## 联通状态和网络位置

先运行本地诊断；当用户询问真实设备可达性、外网状态或网络位置时，再加入只读远端检查：

```bash
roswire --json doctor
roswire --json --profile <profile> doctor --include-remote
```

使用这些只读命令描述网络位置：

```bash
roswire --json --profile <profile> ip address print
roswire --json --profile <profile> interface print
roswire --json --profile <profile> ip route print
roswire --json --profile <profile> tool netwatch print
```

查看公网/DDNS、DNS、设备身份或资源状态时，使用安全的 raw `print` 命令：

```bash
roswire --json --profile <profile> raw /ip/cloud/print
roswire --json --profile <profile> raw /ip/dns/print
roswire --json --profile <profile> raw /system/identity/print
roswire --json --profile <profile> raw /system/resource/print
```

总结联通状态时，优先结合这些证据：

- `doctor --include-remote`：本地配置、依赖、选中的协议、只读远端登录/资源诊断、warnings。
- `ip address print`：接口地址和本地网络位置。
- `interface print`：接口状态、链路状态和接口命名。
- `ip route print`：默认路由、网关、路由是否 active，以及 RouterOS v6/v7 路由字段。
- `tool netwatch print`：已配置的可达性监测及当前状态。
- `/ip/cloud/print`：RouterOS Cloud DDNS/公网地址字段，前提是设备启用并支持。

## 路由表

查看路由表：

```bash
roswire --json --profile <profile> ip route print
```

总结时重点说明默认路由、active/inactive 或 disabled 路由、gateway、distance/scope/target-scope、RouterOS v7 的 routing table 名称，以及是否缺失可疑的默认路由。

## 常用只读设备命令

地址和接口：

```bash
roswire --json --profile <profile> ip address print
roswire --json --profile <profile> interface print
```

防火墙和 NAT：

```bash
roswire --json --profile <profile> ip firewall address-list print
roswire --json --profile <profile> ip firewall filter print
roswire --json --profile <profile> ip firewall nat print
```

WireGuard：

```bash
roswire --json --profile <profile> interface wireguard print
roswire --json --profile <profile> interface wireguard peers print
```

系统和用户：

```bash
roswire --json --profile <profile> system package print
roswire --json --profile <profile> user print
```

Netwatch 和 MAC server：

```bash
roswire --json --profile <profile> tool netwatch print
roswire --json --profile <profile> tool mac-server print
```

## Raw RouterOS 透传

对命令索引未覆盖的高级只读查询使用 `raw`：

```bash
roswire --json --profile <profile> raw /system/resource/print
roswire --json --profile <profile> raw /interface/bridge/print
roswire --json --profile <profile> raw /ip/dhcp-client/print detail=yes
```

`raw` 规则：

- 第一个参数是以 `/` 开头的经典 RouterOS API 路径。
- 额外参数使用 `key=value`。
- roswire 会在错误和日志中隐藏敏感 key 和本地路径。
- 非 `/print` 的 raw 命令需要 `--allow-write`；除非用户明确要求，否则避免使用。

## Script 工作流

把本地 `.rsc` 文件保存为 RouterOS system script，同时不创建 RouterOS 文件：

```bash
roswire --dry-run --json --profile <profile> script put <script-name> --source @setup.rsc
roswire --json --profile <profile> script put <script-name> --source @setup.rsc
```

当可以使用 `--source @<path>` 时，不要把大段脚本内容粘贴到对话里。

## 排错

如果 profile 不存在，运行：

```bash
roswire --json config profiles
```

如果本地配置缺失或状态异常，运行：

```bash
roswire --json doctor
```

如果远端检查失败，汇总 `error_code`、`selected_protocol`、warnings，以及失败发生在远端登录前还是登录后。只有需要更多诊断时才使用 `--debug`，并避免暴露凭据或日志里的 secret 值。
