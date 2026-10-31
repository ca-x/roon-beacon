#!/bin/sh
exec ffmpeg -hide_banner -loglevel error -nostdin -re \
    -f lavfi -i sine=frequency=440:sample_rate=44100 -ac 2 -f s16le pipe:1
