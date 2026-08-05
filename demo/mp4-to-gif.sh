#!/bin/sh
# Derive the small looping GIF (demo/redate.gif) from the demo mp4 for
# inline playback in the GitHub README: github.com will NOT play a
# repo-relative <video>, and user-attachments URLs need a manual
# re-upload on every re-render, so the README embeds a committed GIF
# with ![](demo/redate.gif). The docs site keeps the crisp mp4 (see the
# justfile's site-build).
#
# Runs INSIDE the rendering image, which bundles ffmpeg (the host has no
# native ffmpeg); invoked by `just gifs`. It expects the demo dir mounted
# at /demo. Settings: 1200px wide is retina for the README column
# (~880px) yet keeps the GIF around 1 MB; 8fps and a 64-color diff
# palette suit flat terminal frames; dither=none + diff_mode=rectangle
# maximize inter-frame compression. The mp4 stays the single source of
# truth.
set -e

name=redate
ffmpeg -y -v error -i "/demo/$name.mp4" \
	-vf "fps=8,scale=1200:-1:flags=lanczos,palettegen=max_colors=64:stats_mode=diff" \
	"/demo/pal_$name.png"
ffmpeg -y -v error -i "/demo/$name.mp4" -i "/demo/pal_$name.png" \
	-lavfi "fps=8,scale=1200:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=none:diff_mode=rectangle" \
	"/demo/$name.gif"
rm -f "/demo/pal_$name.png"
