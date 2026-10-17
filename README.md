# DNS Leak Monitor

Monitor outbound plaintext DNS queries on Linux to help detect DNS leaks.

DNS Leak Monitor captures queries on a selected network interface, drops names
matching your exclude list, stores the remaining queries in SQLite, and provides
a local web console. The repository, Cargo package, executable, and systemd
service use the name `dns-leak-monitor`.

## Scope

Use this tool when your DNS policy routes queries through a local resolver such
as AdGuard Home or Pi-hole, and direct DNS traffic on the monitored interface
needs investigation. A captured query is evidence to review against your policy;
it does not automatically prove a leak.

- Linux only; the installer requires systemd.
- Captures UDP/TCP destination port 53. Encrypted DNS such as DoH and DoT is outside
  its scope.
- Excluded names are discarded before storage and cannot be recovered.
- Broad capture without a suitable exclude list can produce a large database.

## Install

Clone this repository and run from its root:

```bash
./install-service.sh
```

Requirements: Rust/Cargo, libpcap (install your distribution's libpcap development
package if needed), systemd, and sudo or root access for installation.

Run the installer as your normal user. It builds the release executable, then
uses sudo to create the `dnswatch` service account, install files, and start the
service. Choose the interface when prompted, or pass it explicitly:

```bash
IFACE=eth0 ./install-service.sh
```

`eth0` is an example; select your actual capture interface. Re-running the
installer keeps an existing configuration.

Installed paths:

| Item | Path |
| --- | --- |
| Executable | `/usr/local/bin/dns-leak-monitor` |
| Configuration | `/etc/dns-leak-monitor/config.toml` |
| Database | `/var/lib/dns-leak-monitor/dns.db` |
| Service | `dns-leak-monitor.service` |

For manual installation, build with `cargo build --release`, copy the executable
from `target/release/dns-leak-monitor`, and use `config.example.toml` and
`systemd/dns-leak-monitor.service` as templates. Create the `dnswatch` account and
data directory, grant it ownership of the data directory, and configure the
capture interface before starting the service.

## Use

Open <http://127.0.0.1:28107>. For a remote machine, use an SSH tunnel:

```bash
ssh -L 28107:127.0.0.1:28107 user@your-server
```

The console supports time filters, name search, pagination, duplicate grouping,
record deletion, and light/dark appearance. It has no authentication; keep the
listener on localhost, or provide appropriate access controls yourself.

Edit `exclude` in `/etc/dns-leak-monitor/config.toml`, then reload it:

```bash
sudo systemctl reload dns-leak-monitor
```

Changing the database path or bind address requires a restart:

```bash
sudo systemctl restart dns-leak-monitor
journalctl -u dns-leak-monitor -f
```

To verify capture, make a DNS query to a resolver permitted by your network policy
on the monitored route, using a name outside your exclude list, and check the
console. Records retain source/destination IP addresses and queried names; keep
runtime configurations, databases, logs, and packet captures out of public uploads.

## Upgrading from the previous name

The previous service was `dns-egress-watch`. Before installing this version,
stop and disable that service to avoid two instances competing for the console
port. Back up the old configuration and database, then migrate them to the new
paths above with the service stopped. Update any explicit `db_path` in the
configuration and preserve `dnswatch` ownership of the data directory. The
installer does not migrate or delete the old installation automatically.

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

### Live capture settings

Open **Capture settings** in the console to select an interface or edit excluded
suffixes (one per line). Saving applies immediately and atomically writes
`config-runtime.toml` beside the main config. These interface/exclude values take
priority over `config.toml`, including after restart or SIGHUP. Delete the runtime
file and reload to return to the main config defaults.

An unavailable/down interface pauses capture without stopping the console.
Retries back off from 5 seconds to a maximum of 60 seconds; changing the interface
interrupts that wait. The console displays capture errors and recovery status.
The monitor keeps the selected interface rather than silently choosing another.

The service user needs write access to the configuration directory. The updated
systemd installer and unit grant this; existing installations should rerun the
installer to update directory ownership and the unit.

### Releases

Push a version tag such as `v0.2.0` (matching Cargo.toml) to publish the GHCR
multi-platform image and a GitHub Release. Branch pushes and pull requests do
not trigger publishing. Releases include Linux x86_64 and ARM64 binary archives
and SHA256SUMS. Binaries built on Ubuntu 24.04 require glibc 2.39 or newer and
libpcap (`libpcap0.8t64` on Ubuntu 24.04). Each archive includes example config
and systemd templates; the installer script builds from a source checkout.

Docker images build in parallel on native x86_64 and ARM64 runners, with
separate architecture caches. Version tags are published only after both builds
succeed, by combining their immutable image digests into a multi-platform manifest.
