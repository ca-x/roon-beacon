#!/bin/bash
set -Eeuo pipefail

image=${1:?image name required}
container=roon-beacon-smoke-$$
volume=roon-beacon-smoke-data-$$
token=synthetic-container-smoke-token
cookie=
synthetic_dir=

cleanup() {
    if [[ -n "$cookie" ]]; then
        rm -f "$cookie"
    fi
    if [[ -n "$synthetic_dir" ]]; then
        rm -rf "$synthetic_dir"
    fi
    docker logs "$container" || true
    docker rm -f "$container" >/dev/null 2>&1 || true
    docker volume rm "$volume" >/dev/null 2>&1 || true
}
trap 'cleanup' EXIT

wait_for_health() {
    for ((attempt = 0; attempt < 60; attempt++)); do
        if docker exec "$container" wget -q -O /dev/null http://127.0.0.1:9339/api/health; then
            return
        fi
        sleep 1
    done
    return 1
}

login() {
    curl --fail --silent --cookie-jar "$cookie" \
        -H 'Content-Type: application/json' -H 'Origin: http://127.0.0.1:9339' \
        --data "{\"token\":\"$token\"}" http://127.0.0.1:9339/api/login
}

wait_for_audio() {
    local required_state=${1:-waiting}
    local status
    for ((attempt = 0; attempt < 30; attempt++)); do
        status=$(curl --fail --silent --cookie "$cookie" http://127.0.0.1:9339/api/audio/status)
        if jq -e --arg state "$required_state" '.enabled and .available and .state == $state and .error == null' <<< "$status"; then
            return
        fi
        sleep 1
    done
    return 1
}

docker volume create "$volume" >/dev/null
docker run -d --name "$container" --network host \
    -v "$volume:/data" -e ROON_BEACON_WEB_TOKEN= "$image" >/dev/null
wait_for_health
token=$(docker exec "$container" cat /data/web-access-token)
test "${#token}" -eq 64
curl --fail --silent http://127.0.0.1:9339/api/health
unauthenticated=$(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:9339/api/audio/status)
test "$unauthenticated" = 401
cookie=$(mktemp)
login
wait_for_audio
network=$(curl --fail --silent --cookie "$cookie" http://127.0.0.1:9339/api/audio/network)
interface=$(jq -er '.interfaces[0].name' <<< "$network")
curl --fail --silent --cookie "$cookie" \
    -H 'Content-Type: application/json' -H 'Origin: http://127.0.0.1:9339' \
    --data "$(jq -cn --arg interface "$interface" '{interface: $interface}')" \
    http://127.0.0.1:9339/api/audio/network
configured=false
for ((attempt = 0; attempt < 30; attempt++)); do
    if docker exec "$container" grep -F -- "interface = \"$interface\";" /data/audio/shairport-sync.conf; then
        configured=true
        break
    fi
    sleep .2
done
test "$configured" = true
sleep 1
wait_for_audio
docker exec "$container" dbus-send --system --type=method_call --print-reply \
    --dest=org.freedesktop.Avahi / org.freedesktop.Avahi.Server.GetState | grep -q 'int32 2'
docker exec "$container" sh -c 'printf persisted > /data/.smoke-persistence'
docker stop --time 15 "$container" >/dev/null
exit_code=$(docker inspect --format '{{.State.ExitCode}}' "$container")
test "$exit_code" != 137
docker start "$container" >/dev/null
wait_for_health
curl --fail --silent --cookie "$cookie" http://127.0.0.1:9339/api/audio/network \
    | jq -e --arg interface "$interface" '.interface == $interface'
wait_for_audio
docker exec "$container" test -d /data/audio
test "$(docker exec "$container" cat /data/.smoke-persistence)" = persisted
docker stop --time 15 "$container" >/dev/null
docker rm "$container" >/dev/null

synthetic_dir=$(mktemp -d)
chmod 755 "$synthetic_dir"
cp docker/synthetic-receiver.sh "$synthetic_dir/shairport-sync"
chmod 755 "$synthetic_dir/shairport-sync"
docker run -d --name "$container" --network host \
    -v "$volume:/data" \
    --mount "type=bind,source=$synthetic_dir,target=/opt/synthetic,readonly" \
    -e 'PATH=/opt/synthetic:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin' \
    -e ROON_BEACON_WEB_TOKEN="$token" "$image" >/dev/null
wait_for_health
login
wait_for_audio streaming
curl --fail --silent --cookie "$cookie" http://127.0.0.1:9339/api/audio/status \
    | jq -e '.pcm_bytes_received > 0 and .bytes_streamed > 0'
curl_status=0
http_status=$(curl --silent --show-error --max-time 3 --cookie "$cookie" \
    --output "$synthetic_dir/stream.mp3" --write-out '%{http_code}' \
    http://127.0.0.1:9339/api/audio/stream) || curl_status=$?
test "$http_status" = 200
test "$curl_status" -eq 28
test "$(wc -c < "$synthetic_dir/stream.mp3")" -gt 24000
docker cp "$synthetic_dir/stream.mp3" "$container:/tmp/synthetic.mp3"
docker exec "$container" ffprobe -v error -select_streams a:0 \
    -show_entries stream=codec_name,sample_rate,channels -of json /tmp/synthetic.mp3 \
    | jq -e '.streams[0] | .codec_name == "mp3" and .sample_rate == "44100" and .channels == 2'
docker exec "$container" ffmpeg -hide_banner -loglevel error \
    -i /tmp/synthetic.mp3 -t 1 -f s16le pipe:1 \
    | od -An -v -t d2 \
    | awk '{ for (field = 1; field <= NF; field++) if ($field != 0) nonzero = 1 } END { exit !nonzero }'
docker stop --time 15 "$container" >/dev/null
exit_code=$(docker inspect --format '{{.State.ExitCode}}' "$container")
test "$exit_code" != 137
