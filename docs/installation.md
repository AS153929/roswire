# 安装 RosWire

> 最后更新：2026-05-19
> 适用状态：MVP / Beta 候选。生产级稳定版仍需完成 RouterOS 真机/CHR 验收矩阵（#60）。

本文说明如何从 GitHub Release 安装 `roswire`，如何校验下载产物，以及如何确认二进制可以独立运行。

## 快速安装脚本

Linux 用户可以使用一行命令从 latest GitHub Release 安装：

```bash
curl -fsSL https://raw.githubusercontent.com/AS153929/roswire/main/scripts/install.sh | sh
```

脚本会自动识别 Linux x86_64 / arm64，下载对应 release 产物和 `checksums.txt`，校验 SHA256，然后安装到 `/usr/local/bin/roswire`。Linux release 产物使用 musl 静态目标构建，不依赖目标机器的 glibc。如果当前用户无写入权限，脚本会尝试通过 `sudo` 执行安装。

可通过环境变量覆盖默认行为：

```bash
curl -fsSL https://raw.githubusercontent.com/AS153929/roswire/main/scripts/install.sh | ROSWIRE_VERSION=v0.0.3 sh
curl -fsSL https://raw.githubusercontent.com/AS153929/roswire/main/scripts/install.sh | ROSWIRE_INSTALL_DIR="$HOME/.local/bin" sh
```

| 变量 | 说明 |
| --- | --- |
| `ROSWIRE_VERSION` | 指定 release tag，例如 `v0.0.3`；默认使用 latest |
| `ROSWIRE_INSTALL_DIR` | 指定安装目录；默认 `/usr/local/bin` |
| `ROSWIRE_REPO` | 指定 GitHub 仓库；默认 `AS153929/roswire` |
| `ROSWIRE_VERIFY=0` | 跳过 SHA256 校验；不推荐 |

macOS 预编译产物暂不发布，因此快速安装脚本目前不支持 macOS；请使用“从源码安装”。

## Cargo 安装

如果本机已经安装 Rust stable 工具链，可以在 crate 发布到 crates.io 后直接安装：

```bash
cargo install roswire --locked
```

从仓库源码安装当前分支：

```bash
cargo install --git https://github.com/AS153929/roswire --locked
```

在本地 checkout 中安装当前源码：

```bash
cargo install --path . --locked
```

Cargo 默认把二进制安装到 `~/.cargo/bin`。请确认该目录已经加入 `PATH`。

## 选择平台产物

Release 产物命名约定：

| 平台 | 文件 |
| --- | --- |
| Linux x86_64 | `roswire-linux-amd64.tar.gz` |
| Linux arm64 | `roswire-linux-arm64.tar.gz` |
| Windows x86_64 | `roswire-windows-amd64.zip` |
| 校验和 | `checksums.txt` |

Linux 归档内的 `roswire` 是 musl 静态链接二进制，适合 glibc 版本较旧或没有 glibc 的发行环境。Release workflow 会对 Linux 二进制做 UPX 压缩以降低下载和落盘体积。

macOS 预编译产物暂不发布；macOS 用户请先按“从源码安装”构建本机二进制。

## Linux 安装

1. 下载对应平台的归档和校验和文件。

```bash
curl -L https://github.com/AS153929/roswire/releases/latest/download/roswire-linux-amd64.tar.gz -o roswire-linux-amd64.tar.gz
curl -L https://github.com/AS153929/roswire/releases/latest/download/checksums.txt -o checksums.txt
```

1. 校验 SHA256。

```bash
sha256sum -c checksums.txt --ignore-missing
```

1. 解压并安装到 PATH。

```bash
tar -xzf roswire-linux-amd64.tar.gz
chmod +x roswire
sudo install -m 0755 roswire /usr/local/bin/roswire
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

设备、连接和传输字段建议写入 profile；secret 只保存引用。`ROS_*` 单设备环境变量入口已移除，避免多设备场景误连。

```bash
export ROSWIRE_STUDIO_PASSWORD="replace-with-secret"
roswire config init --json
roswire config device add studio \
  host=192.168.88.1 \
  user=admin \
  protocol=auto \
  transfer=ssh \
  ssh_host_key=SHA256:replace-with-routeros-host-key \
  allow_from=203.0.113.10/32 \
  --json
roswire config secret set studio password type=env env=ROSWIRE_STUDIO_PASSWORD --json
```

随后可以运行只读命令：

```bash
roswire interface print --json
```

文件传输需要 SSH host key 指纹；如果 profile 已设置 `ssh_host_key` 和 `allow_from`，dry-run 可以直接读取 profile。也可以用命令行参数临时覆盖：

```bash
roswire file upload ./setup.rsc flash/setup.rsc --dry-run --json
roswire file upload ./setup.rsc flash/setup.rsc --dry-run --ssh-host-key SHA256:replace-with-routeros-host-key --allow-from 203.0.113.10/32 --json
```

## 平台依赖提示

- macOS：Keychain 通常随系统可用；发布前 keychain 原生 smoke 见 [`keychain-smoke.md`](keychain-smoke.md)。
- Linux：如果使用 keychain secret，需要 Secret Service / D-Bus / libsecret 运行环境；无可用后端时会返回 `SECRET_BACKEND_UNAVAILABLE`。
- Windows：Credential Manager 路径需在目标用户会话下验证；无可用后端时会返回 `SECRET_BACKEND_UNAVAILABLE`。

## 卸载

Linux：

```bash
sudo rm -f /usr/local/bin/roswire
```

Windows：删除解压目录，并从 `PATH` 中移除该目录。

本地配置默认在 `~/.roswire/`。删除该目录会移除本地 profile、缓存和日志；不会自动删除 OS keychain 中由用户创建的 credential。
