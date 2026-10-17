#!/bin/sh
set -eu

# Keep the template outside the mount point: a bind mount hides image files.
config_dir=/etc/dns-leak-monitor
if [ ! -e "$config_dir/config.toml" ]; then
    mkdir -p "$config_dir"
    cp /usr/share/dns-leak-monitor/config.toml "$config_dir/config.toml"
fi

exec /usr/local/bin/dns-leak-monitor "$@"
