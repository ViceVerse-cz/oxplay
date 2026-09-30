#!/bin/sh
set -eu
mkdir -p artifacts
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=1920x1080:rate=60 -f lavfi -i sine=frequency=440:sample_rate=48000 -t 90 -c:v libx264 -preset fast -crf 20 -pix_fmt yuv420p -c:a aac -b:a 128k -y artifacts/local-1080p60.mp4
cat > artifacts/local.srt <<'SUB'
1
00:00:01,000 --> 00:00:08,000
Oxplay local subtitle validation

2
00:00:20,000 --> 00:00:28,000
Seek and subtitle composition
SUB
