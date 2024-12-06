#!/bin/bash

for file in `find prenormalized -type f -exec bash -c '[[ "$( file -bi "$1" )" == audio/* ]]' bash {} \; -print`; do
  out="sounds/${file#prenormalized/}.wav"

  if [ ! -f "$out"]; then
    echo "Normalizing file $file"
    mkdir -p "$(dirname $out)"
    ffmpeg-normalize $file -o $out
  else
    echo "Skipping existing file $file"
  fi
done
