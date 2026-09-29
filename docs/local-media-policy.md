# Local media input policy

The in-app picker accepts explicitly selected regular MP4/M4V/MOV and
Matroska/WebM files after bounded header validation on its worker. Subtitle
selection is restricted to bounded UTF-8 SRT/WebVTT files for the current local
video. Paths are not saved to local history or exposed as share links.

Header validation alone is not a parser boundary: a file can change between
validation and native opening. Every absolute local path submitted by the media
boundary therefore receives entry-scoped `demuxer=lavf`, `sub-demuxer=lavf` and
`demuxer-lavf-o=format_whitelist=[mov,matroska,webm,avi,wav,flac,mp3,ogg,srt,ass,webvtt]`.
The additional audio/container formats retain existing explicit CLI/diagnostic
inputs; they are not additional picker choices. Other containers can now fail
instead of being probed by another demuxer. Initial and subsequently attached
subtitles inherit the current entry's policy. Replacement remote loads do not
receive the local whitelist; their existing policy-owned compressed-stream path
remains separate.

The options were checked against official
[mpv v0.41.0 options](https://github.com/mpv-player/mpv/blob/v0.41.0/DOCS/man/options.rst),
[its option parser](https://github.com/mpv-player/mpv/blob/v0.41.0/options/m_option.c),
and [its lavf demuxer](https://github.com/mpv-player/mpv/blob/v0.41.0/demux/demux_lavf.c).
The bracketed value is mpv's key/value quoting syntax, preserving the comma list
as one FFmpeg option. The existing global `access-references=no` installs lavf's
rejecting nested `io_open` callback. FFmpeg's
[`avformat_open_input`](https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavformat/demux.c)
checks `format_whitelist` before invoking the selected format's header reader.
Forcing lavf prevents mpv playlist/EDL/image-sequence demuxers from taking over;
the whitelist excludes HLS/DASH and other reference formats.

This is a source-reviewed restriction, not a sandbox or a claim that hostile
media is safe. Runtime malformed-container, file-replacement and negative-egress
qualification of this addition has not run in the current feature pass. The
existing native punctuated-path WAV/subtitle regression is compiled by all-target
Clippy; no new performance or native tests are requested for this pass.
