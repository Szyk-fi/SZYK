#!/usr/bin/env bash
# Downloads every track of a SoundCloud playlist (or set) as WAV, numbered in
# playlist order, with yt-dlp.
#
#   tools/soundcloud_playlist.sh <playlist-url> [output-folder]
#
# Needs yt-dlp and ffmpeg (brew install yt-dlp ffmpeg). Only use it on tracks
# you have the right to keep: your own uploads, tracks whose owner enabled
# downloads, or material you bought; SoundCloud's terms don't allow
# downloading anything else. Private playlists need the secret link as given.
set -euo pipefail

url="${1:?usage: $0 <playlist-url> [output-folder]}"
out="${2:-$HOME/Documents/Audacity4/soundcloud}"

command -v yt-dlp >/dev/null || { echo "yt-dlp not found: brew install yt-dlp" >&2; exit 1; }
command -v ffmpeg >/dev/null || { echo "ffmpeg not found: brew install ffmpeg" >&2; exit 1; }

mkdir -p "$out"
# %(playlist_index|0)03d keeps playlist order; --download-archive lets you re-run
# the script to pick up only new tracks; --sleep-interval is polite to the site.
yt-dlp \
  --ffmpeg-location "$(dirname "$(command -v ffmpeg)")" \
  --extract-audio --audio-format wav \
  --output "$out/%(playlist_index|0)03d - %(title)s.%(ext)s" \
  --download-archive "$out/.downloaded.txt" \
  --sleep-interval 1 --max-sleep-interval 3 \
  --no-overwrites --restrict-filenames \
  "$url"

echo "Saved to $out"
