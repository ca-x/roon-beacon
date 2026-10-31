#!/bin/bash
set -Eeuo pipefail

pids=()

cleanup() {
    trap - EXIT INT TERM
    if ((${#pids[@]})); then
        kill -TERM "${pids[@]}" 2>/dev/null || true
        for ((attempt = 0; attempt < 100; attempt++)); do
            alive=0
            for pid in "${pids[@]}"; do
                if kill -0 "$pid" 2>/dev/null; then
                    alive=1
                fi
            done
            if ((alive == 0)); then
                break
            fi
            sleep 0.1
        done
        kill -KILL "${pids[@]}" 2>/dev/null || true
        for pid in "${pids[@]}"; do
            wait "$pid" 2>/dev/null || true
        done
    fi
}

trap 'cleanup' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

install -d -m 755 /run/dbus /run/avahi-daemon
install -d -m 700 -o shairport-sync -g shairport-sync /data
dbus-uuidgen --ensure=/etc/machine-id
ln -sf /etc/machine-id /var/lib/dbus/machine-id
export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket
rm -f /run/dbus/pid /run/dbus/system_bus_socket /run/avahi-daemon/pid

dbus-daemon --system --nofork --nopidfile --address="$DBUS_SYSTEM_BUS_ADDRESS" &
pids+=("$!")
ready=0
for ((attempt = 0; attempt < 50; attempt++)); do
    if dbus-send --system --type=method_call --print-reply \
        --dest=org.freedesktop.DBus / org.freedesktop.DBus.ListNames >/dev/null 2>&1; then
        ready=1
        break
    fi
    kill -0 "${pids[0]}" 2>/dev/null || break
    sleep 0.1
done
if ((ready == 0)); then
    printf '%s\n' 'Private system D-Bus failed to start.' >&2
    exit 1
fi

avahi-daemon --no-chroot --no-rlimits --file=/etc/avahi/avahi-daemon.conf &
pids+=("$!")
ready=0
for ((attempt = 0; attempt < 100; attempt++)); do
    if dbus-send --system --type=method_call --print-reply \
        --dest=org.freedesktop.Avahi / org.freedesktop.Avahi.Server.GetState 2>/dev/null \
        | grep -q 'int32 2'; then
        ready=1
        break
    fi
    kill -0 "${pids[1]}" 2>/dev/null || break
    sleep 0.1
done
if ((ready == 0)); then
    printf '%s\n' 'Private Avahi failed to become ready. Use Linux host networking for LAN discovery.' >&2
    exit 1
fi

setpriv --reuid=shairport-sync --regid=shairport-sync --init-groups \
    /usr/local/bin/roon-beacon "$@" &
pids+=("$!")
status=0
wait -n "${pids[@]}" || status=$?
cleanup
exit "$status"
