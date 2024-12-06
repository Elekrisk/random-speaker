#!/bin/bash

for file in `find prenormalized -type f -exec bash -c '[[ "$( file -bi "$1" )" == audio/* ]]' bash {} \; -print`; do
  echo $file
  out="sounds/${file#prenormalized/}.wav"

  mkdir -p "$(dirname $out)"
  ffmpeg-normalize $file -o $out
done
