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

  

## 目录

- [目录](#目录)
- [1. 它解决什么问题](#1-它解决什么问题)
- [2. 安装](#2-安装)
  - [2.1 Docker Compose（推荐）](#21-docker-compose推荐)
  - [2.2 systemd 服务](#22-systemd-服务)
  - [2.3 从源码构建](#23-从源码构建)
- [3. 自定义配置](#3-自定义配置)
- [4. 使用建议](#4-使用建议)
- [5. 项目结构](#5-项目结构)
- [6. 开发](#6-开发)

## 1. 它解决什么问题

本机 DNS 服务可能已经把常规请求转成了 DoH，或者通过代理发出dns请求，但仍可能发生dns泄露的情况：

- 个人的配置错误；
- 某个程序自行指定了公共 DNS 服务器；
- 某个容器、虚拟机或网络命名空间没有指定的网络链路；
- 某台设备或客户端临时恢复了直连 DNS。

这时候你需要知道什么有没有泄露，泄露了什么，才能升级加固配置。一般会使用`tcpdump`进行临时的抓包测试，但是这种方式没办法**常驻**、**记录**、**筛选**。因此有了本项目`dns-leak-monitor`通过常驻服务来做这些事情。

## 2. 安装

### 2.1 Docker Compose（推荐）

需要 Linux Docker Engine，并允许容器使用 host network、`NET_RAW` 和 `NET_ADMIN`。创建 `compose.yml`：

```yaml
# Linux host; run: docker compose up -d
services:
  dns-leak-monitor:
    image: ghcr.io/ctxinf/dns-leak-monitor:latest
    network_mode: host
    user: "0:0"
    cap_drop:
      - ALL
    cap_add:
      - NET_RAW
      - NET_ADMIN
    security_opt:
      - no-new-privileges:true
    read_only: true
    restart: unless-stopped
    stop_grace_period: 10s
    volumes:
      # - ./config:/etc/dns-leak-monitor:Z
      - dns-leak-monitor-data:/var/lib/dns-leak-monitor
    logging:
      driver: json-file
      options:
        max-size: "10m"
        max-file: "3"

volumes:
  dns-leak-monitor-data:
    name: dns-leak-monitor-data
```

启动服务：

```bash
docker compose up -d
```

查看日志或停止服务：

```bash
docker compose logs -f
docker compose down
```

### 2.2 systemd 服务

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

### 2.3 从源码构建

```bash
cargo build --release --locked
```

需要 Rust stable 和 libpcap 开发包。生成的二进制位于 `target/release/dns-leak-monitor`。可以参考 [`config.example.toml`](config.example.toml)、[`deploy/systemd/dns-leak-monitor.service`](deploy/systemd/dns-leak-monitor.service) 和安装脚本完成手动部署。

## 3. 自定义配置

配置文件放置位置：

- Docker Compose：`compose.yml` 同目录下的 `config/config.toml`，挂载到容器内的 `/etc/dns-leak-monitor/config.toml`。需要先取消 Compose 文件中 `- ./config:/etc/dns-leak-monitor:Z` 的注释。
- systemd：`/etc/dns-leak-monitor/config.toml`。
- 从源码运行：通过 `--config` 指定配置文件路径。

Docker 下，启用挂载后先执行 `docker compose up -d`，让容器自动创建配置目录和文件，再编辑 `config/config.toml`。不要提前用 `mkdir` 创建目录，否则可能产生权限问题。挂载整个配置目录可以让控制台保存的配置在容器重建后保留。

```toml
# 必填：要监听的主机网卡，改成实际承载出站 DNS 流量的网卡。
interface = "eth0"

# SQLite 数据库路径，指容器内或 systemd 所在主机上的路径。
db_path = "/var/lib/dns-leak-monitor/dns.db"

# Web 控制台监听地址；Docker 使用 host network，直接监听主机回环地址。
bind = "127.0.0.1:28107"

# 记录保留天数，至少为 1。
retention_days = 90

# 排除的域名后缀，不区分大小写；example.com 也会匹配 a.example.com。
# 匹配的查询不会入库。以下默认值用于排除反向 DNS 查询。
exclude = ["in-addr.arpa", "ip6.arpa"]
```

修改后，Docker 执行 `docker compose restart`，systemd 执行 `sudo systemctl restart dns-leak-monitor`。控制台保存的网卡和排除名单位于同目录的 `config-runtime.toml`，会优先于 `config.toml` 中的对应配置。

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
