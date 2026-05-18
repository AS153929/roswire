# 安装 RosWire

> 最后更新：2026-05-18
> 适用状态：MVP / Beta 候选。生产级稳定版仍需完成 RouterOS 真机/CHR 验收矩阵（#60）。

本文说明如何从 GitHub Release 安装 `roswire`，如何校验下载产物，以及如何确认二进制可以独立运行。

## 选择平台产物

Release 产物命名约定：

| 平台 | 文件 |
| --- | --- |
| Linux x86_64 | `roswire-linux-amd64.tar.gz` |
| Linux arm64 | `roswire-linux-arm64.tar.gz` |
| macOS Intel | `roswire-macos-amd64.tar.gz` |
| macOS Apple Silicon | `roswire-macos-arm64.tar.gz` |
| Windows x86_64 | `roswire-windows-amd64.zip` |
| 校验和 | `checksums.txt` |

## Linux / macOS 安装

1. 下载对应平台的归档和校验和文件。

```bash
curl -L https://github.com/AS153929/roswire/releases/latest/download/roswire-linux-amd64.tar.gz -o roswire-linux-amd64.tar.gz
curl -L https://github.com/AS153929/roswire/releases/latest/download/checksums.txt -o checksums.txt
```

macOS 请把文件名换成 `roswire-macos-amd64.tar.gz` 或 `roswire-macos-arm64.tar.gz`。

1. 校验 SHA256。

```bash
sha256sum -c checksums.txt --ignore-missing
```

macOS 如果没有 GNU `sha256sum`，可用：

```bash
shasum -a 256 roswire-macos-arm64.tar.gz
cat checksums.txt
```

确认输出 hash 与 `checksums.txt` 中对应行一致。

1. 解压并安装到 PATH。

```bash
tar -xzf roswire-linux-amd64.tar.gz
chmod +x roswire
sudo install -m 0755 roswire /usr/local/bin/roswire
```

macOS 如遇 Gatekeeper quarantine，可在确认校验和后执行：

```bash
xattr -d com.apple.quarantine /usr/local/bin/roswire 2>/dev/null || true
```

1. 验证二进制可运行。

```bash
roswire doctor --json
```

`doctor` 默认只做本地检查，不访问 RouterOS。

## Windows 安装

1. 下载 `roswire-windows-amd64.zip` 和 `checksums.txt`。
1. 在 PowerShell 中校验 SHA256：

```powershell
$hash = (Get-FileHash .\roswire-windows-amd64.zip -Algorithm SHA256).Hash.ToLower()
Select-String -Path .\checksums.txt -Pattern $hash
```

1. 解压 zip，并把包含 `roswire.exe` 的目录加入 `PATH`。

```powershell
Expand-Archive .\roswire-windows-amd64.zip -DestinationPath .\roswire
.\roswire\roswire.exe doctor --json
```

## 从源码安装

需要 Rust stable 工具链。

```bash
git clone https://github.com/AS153929/roswire.git
cd roswire
cargo build --release --locked
./target/release/roswire doctor --json
```

## 最小配置示例

建议优先使用环境变量或 profile secret，避免把密码放进 shell history。

```bash
export ROS_HOST="192.168.88.1"
export ROS_USER="admin"
export ROS_PASSWORD="replace-with-secret"
export ROS_PROTOCOL="auto"
export ROS_TRANSFER="ssh"
export ROS_SSH_HOST_KEY="SHA256:replace-with-routeros-host-key"
```

随后可以运行只读命令：

```bash
roswire interface print --json
```

文件传输需要 SSH host key 指纹；如果要临时确保 SSH 服务开启，应显式提供窄白名单：

```bash
roswire file upload ./setup.rsc flash/setup.rsc --dry-run --ssh-host-key "$ROS_SSH_HOST_KEY" --allow-from 203.0.113.10/32 --json
```

## 平台依赖提示

- macOS：Keychain 通常随系统可用；发布前 keychain 原生 smoke 见 [`keychain-smoke.md`](keychain-smoke.md)。
- Linux：如果使用 keychain secret，需要 Secret Service / D-Bus / libsecret 运行环境；无可用后端时会返回 `SECRET_BACKEND_UNAVAILABLE`。
- Windows：Credential Manager 路径需在目标用户会话下验证；无可用后端时会返回 `SECRET_BACKEND_UNAVAILABLE`。

## 卸载

Linux/macOS：

```bash
sudo rm -f /usr/local/bin/roswire
```

Windows：删除解压目录，并从 `PATH` 中移除该目录。

本地配置默认在 `~/.roswire/`。删除该目录会移除本地 profile、缓存和日志；不会自动删除 OS keychain 中由用户创建的 credential。
