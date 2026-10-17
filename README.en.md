<h1 align="center">
  <img src="static/favicon.svg" width="48" height="48" alt="DNS Leak Monitor" valign="middle">
  DNS Leak Monitor
</h1>

<p align="center"><a href="README.md">中文</a> · English</p>

<p align="center">
  Monitor outbound plaintext DNS queries on Linux
</p>

<p align="center">
  <img src="docs/screenshot.png" alt="DNS Leak Monitor console" width="900">
</p>

> [!WARNING]
> **Prerequisite**
>
> This tool is intended for environments where outbound plaintext <code>:53</code> traffic is occasional.
> Do not use it for a DNS direct-out setup where most queries go straight to upstream resolvers: the monitor will continuously record a large volume of DNS queries, which is outside this tool's intended use.

**Features:**

- Watches outbound UDP/TCP `53` requests on one explicitly selected interface; it fails when the interface is missing instead of guessing.
- Excludes known-good names by suffix before storage, leaving the database focused on records to investigate.
- Provides a local web console with time filters, name search, pagination, duplicate collapsing, auto-refresh, and deletion of test records.
- Shows the source address, DNS destination, query name, type, and local time so you can trace the request back to its path.
- Lets you change the interface and exclusion list from the console; capture pauses and retries when the selected interface is unavailable.
- Stores records in SQLite with a 90-day default retention period; the console binds to `127.0.0.1:28107` by default.

<br>

**What it cannot do:**

- It can monitor only one interface at a time; it does not combine traffic from multiple interfaces.
- It only sees plaintext DNS sent to destination port `53`. DoH, DoT, and other encrypted DNS are outside its scope, and it cannot inspect the names inside DoH traffic.
- It does not block, rewrite, or repair DNS requests. After finding a record, you still need to inspect the client, container, or local DNS service configuration.
- A `:53` record is not automatically proof of a leak. It may be allowed by policy or come from a legitimate component that is not yet in the exclusion list.
- Excluded names are dropped during capture and cannot be recovered from the database. Keep the list narrow until a name is confirmed safe.
- This is a Linux tool. Packet capture requires libpcap and `CAP_NET_RAW`/`CAP_NET_ADMIN`; the Docker deployment also requires host networking on a Linux host.
- The web console has no login authentication. It listens on localhost by default; use an SSH tunnel or provide your own access control for remote access.

## Table of contents

- [Table of contents](#table-of-contents)
- [1. The problem it solves](#1-the-problem-it-solves)
- [3. Installation](#3-installation)
  - [3.1 Docker Compose (recommended)](#31-docker-compose-recommended)
  - [3.2 systemd service](#32-systemd-service)
  - [3.3 Build from source](#33-build-from-source)
- [4. Recommended workflow](#4-recommended-workflow)
- [5. Repository layout](#5-repository-layout)
- [6. Development](#6-development)

## 1. The problem it solves

Your local DNS service may already turn normal requests into DoH, while some traffic still bypasses it:

- an application chooses a public resolver directly;
- a container, VM, or network namespace uses a different resolver configuration;
- a device or client temporarily falls back to plaintext DNS;
- a network change introduces a new bypass.

If the host produces a constant stream of `:53` traffic, the monitor will be noisy and the DNS takeover itself still needs work. The useful operating state is one where most requests go through your local `in:dns out:doh` service and direct `:53` requests are occasional exceptions.

## 3. Installation

### 3.1 Docker Compose (recommended)

You need Docker Engine on Linux and permission for host networking plus `NET_RAW` and `NET_ADMIN`. From the repository root:

```bash
git clone https://github.com/ctxinf/dns-leak-monitor.git
cd dns-leak-monitor
IMAGE=ghcr.io/ctxinf/dns-leak-monitor:latest \
  docker compose -f deploy/docker/compose.yaml up -d --no-build
```

Set `interface` in `deploy/docker/config/config.toml` to the host interface that carries the DNS traffic. The configuration directory is bind-mounted into the container, so interface and exclusion changes made by the console survive container recreation.

To build the image from the checkout instead:

```bash
docker compose -f deploy/docker/compose.yaml up -d --build
```

View logs or stop the service:

```bash
docker compose -f deploy/docker/compose.yaml logs -f
docker compose -f deploy/docker/compose.yaml down
```

### 3.2 systemd service

Use this on a Linux host with Rust/Cargo, the libpcap development package, and systemd. The script builds the release binary, creates a dedicated service user, installs the unit, and starts it:

```bash
git clone https://github.com/ctxinf/dns-leak-monitor.git
cd dns-leak-monitor
IFACE=eth0 ./deploy/systemd/install-service.sh
```

Replace `eth0` with the interface that actually carries outbound DNS traffic. You can omit `IFACE` and select an interface interactively. Run the script as your normal user; it asks for sudo only during installation.

After installation:

```bash
# Open locally
http://127.0.0.1:28107

# On a remote host: create a tunnel from your local machine,
# then open the same URL locally
ssh -L 28107:127.0.0.1:28107 user@your-server
```

After editing `/etc/dns-leak-monitor/config.toml`, reload for exclusion-list changes. Restart for interface, bind-address, or database-path changes:

```bash
sudo systemctl reload dns-leak-monitor
sudo systemctl restart dns-leak-monitor
journalctl -u dns-leak-monitor -f
```

### 3.3 Build from source

```bash
cargo build --release --locked
```

You need stable Rust and the libpcap development package. The binary is written to `target/release/dns-leak-monitor`. For a manual deployment, use [`config.example.toml`](config.example.toml), [`deploy/systemd/dns-leak-monitor.service`](deploy/systemd/dns-leak-monitor.service), and the installer script as references.

## 4. Recommended workflow

1. Confirm that your local DNS service handles the main DNS entry point and uses DoH upstream.
2. You can add confirmed-good, high-volume names or reverse-DNS domains to `exclude`, such as `in-addr.arpa` and `ip6.arpa`.
3. Review the remaining `:53` requests and trace each source address to the relevant process, container, VM, or client configuration.
4. Run one deliberate test query to verify the capture path, then delete the test record. Keep real leads until the investigation is complete.

Exclusions use DNS suffix matching: `example.com` also excludes `a.example.com`. Records are retained for 90 days by default. The database and logs may contain hostnames and addresses, so apply access controls and backup rules appropriate for your environment.

## 5. Repository layout

- `src/`: capture, DNS parsing, exclusion, SQLite storage, and web console.
- `deploy/systemd/`: systemd unit and installer.
- `deploy/docker/`: Dockerfile, Compose file, and container defaults.
- `config.example.toml`: host deployment configuration example.

## 6. Development

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```
