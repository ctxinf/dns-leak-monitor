<h1 align="center">
  <img src="static/favicon.svg" width="48" height="48" alt="DNS Leak Monitor" valign="middle">
  DNS Leak Monitor
</h1>

<p align="center"><a href="README.en.md">English</a> · 中文</p>


<p align="center">
  在 Linux 上监控DNS泄露(出站明文 DNS 查询)
</p>

<p align="center">
  <img src="docs/screenshot.png" alt="DNS Leak Monitor 控制台" width="900">
</p>


> [!WARNING]
> **使用前提**
>
> 本工具适用于出站明文 <code>:53</code> 请求只偶尔出现的环境。
> 不适用于 DNS 直出场景：如果大多数 DNS 请求都直接发往上游，监控结果会持续记录海量dns查询请求，这不属于本工具的目标使用场景。




**特性:**

- 监听一个指定网卡上的出站 UDP/TCP `53` 请求；不猜测网卡，配置缺失时直接报错。
- 按 DNS 名称后缀排除已知正常流量，排除项在写入前丢弃，让数据库只保留待调查记录。
- 本地 Web 控制台支持时间筛选、名称搜索、分页、重复项折叠、自动刷新和删除测试记录。
- 显示源地址、目标 DNS 地址、查询名称、类型和本地时间，方便回溯是哪条路径发出的请求。
- 网卡和排除名单可以在控制台修改；网卡暂时不可用时暂停捕获并自动重试。
- SQLite 保存记录，默认保留 90 天；控制台默认只绑定 `127.0.0.1:28107`。

<br>

**不能做：**

- 同时只能监听一张网卡；它不会把多张网卡的流量合并监控。
- 只看明文 DNS 的目标端口 `53`，不能看到 DoH、DoT 或其他加密 DNS，也不能解释已经被 DoH 加密的请求内容。
- 不会主动阻止、改写或修复 DNS 请求；发现记录后仍需要你检查客户端、容器或本机 DNS 服务的配置。
- 一条 `:53` 记录本身不等于泄漏证明，它可能是策略允许的请求，也可能来自你尚未加入排除名单的正常组件。
- 排除项会在捕获阶段直接丢弃，之后无法从数据库恢复；不要过早加入过宽的域名。
- 这是 Linux 工具。抓包依赖 libpcap 和 `CAP_NET_RAW`/`CAP_NET_ADMIN`；Docker 方式也要求 Linux 主机网络模式。
- Web 控制台没有登录认证；默认只监听本机，远程访问请使用 SSH 隧道或自行提供访问控制。
  

## 目录

- [目录](#目录)
- [1. 它解决什么问题](#1-它解决什么问题)
- [3. 安装](#3-安装)
  - [3.1 Docker Compose（推荐）](#31-docker-compose推荐)
  - [3.2 systemd 服务](#32-systemd-服务)
  - [3.3 从源码构建](#33-从源码构建)
- [4. 使用建议](#4-使用建议)
- [5. 项目结构](#5-项目结构)
- [6. 开发](#6-开发)

## 1. 它解决什么问题

本机 DNS 服务可能已经把常规请求转成了 DoH，但以下流量仍可能绕过它：

- 某个程序自行指定了公共 DNS 服务器；
- 某个容器、虚拟机或网络命名空间使用了另一套解析配置；
- 某台设备或客户端临时恢复了直连 DNS；
- 网络配置改变后，原本不明显的旁路开始出现。

如果整台机器上每秒都有大量 `:53` 请求，监控器会产生噪声，也说明 DNS 接管本身还没有完成。它更适合这样的状态：大部分请求已经走本机的 `in:dns out:doh` 服务，直连 `:53` 只应当偶尔出现。



## 3. 安装

### 3.1 Docker Compose（推荐）

需要 Linux Docker Engine，并允许容器使用 host network、`NET_RAW` 和 `NET_ADMIN`。在仓库根目录执行：

```bash
git clone https://github.com/ctxinf/dns-leak-monitor.git
cd dns-leak-monitor
IMAGE=ghcr.io/ctxinf/dns-leak-monitor:latest \
  docker compose -f deploy/docker/compose.yaml up -d --no-build
```

配置文件位于 `deploy/docker/config/config.toml`，其中的 `interface` 改成主机上实际要监听的网卡。配置目录会挂载到容器内，因此控制台保存的网卡和排除名单可以在容器重建后保留。

如果要从当前源码构建镜像：

```bash
docker compose -f deploy/docker/compose.yaml up -d --build
```

查看日志或停止服务：

```bash
docker compose -f deploy/docker/compose.yaml logs -f
docker compose -f deploy/docker/compose.yaml down
```

### 3.2 systemd 服务

适合已经安装 Rust/Cargo、libpcap 开发包和 systemd 的 Linux 主机。脚本会构建 release 二进制、创建专用用户、安装服务并启动它：

```bash
git clone https://github.com/ctxinf/dns-leak-monitor.git
cd dns-leak-monitor
IFACE=eth0 ./deploy/systemd/install-service.sh
```

把 `eth0` 替换成真正承载出站 DNS 流量的网卡。也可以不设置 `IFACE`，让脚本交互式选择。脚本以普通用户运行，安装阶段会请求 sudo；不要先用 sudo 启动脚本。

安装后：

```bash
# 本机打开
http://127.0.0.1:28107

# 远程主机：在本地建立隧道后打开同一个地址
ssh -L 28107:127.0.0.1:28107 user@your-server
```

编辑 `/etc/dns-leak-monitor/config.toml` 后，修改排除名单可以 reload；修改网卡、监听地址或数据库路径需要 restart：

```bash
sudo systemctl reload dns-leak-monitor
sudo systemctl restart dns-leak-monitor
journalctl -u dns-leak-monitor -f
```

### 3.3 从源码构建

```bash
cargo build --release --locked
```

需要 Rust stable 和 libpcap 开发包。生成的二进制位于 `target/release/dns-leak-monitor`。可以参考 [`config.example.toml`](config.example.toml)、[`deploy/systemd/dns-leak-monitor.service`](deploy/systemd/dns-leak-monitor.service) 和安装脚本完成手动部署。

## 4. 使用建议

1. 先确认本机的 DNS 服务确实承担主要入口，并通过 DoH 访问上游。
2. 可以把已经确认正常的高频域名或反向解析域名加入 `exclude`，例如 `in-addr.arpa` 和 `ip6.arpa`。
3. 观察控制台中剩余的 `:53` 请求，沿着源地址检查对应的进程、容器、虚拟机或客户端配置。
4. 用一次明确的测试查询验证捕获链路，确认后删除测试记录；真实线索应保留到调查完成。

排除名单匹配域名后缀。例如 `example.com` 也会排除 `a.example.com`。记录默认保留 90 天，数据库和日志可能包含主机名及地址，请按你的环境进行访问控制和备份管理。

## 5. 项目结构

- `src/`：抓包、DNS 解析、排除、SQLite 存储和 Web 控制台。
- `deploy/systemd/`：systemd 单元和安装脚本。
- `deploy/docker/`：Dockerfile、Compose 文件和容器默认配置。
- `config.example.toml`：主机部署的配置示例。

## 6. 开发

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```
