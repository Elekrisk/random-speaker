
for file in `find sounds -type f` ; do
  in="prenormalized/${file#sounds/}"
  in="${in%.wav}"

  if [ ! -f "$in" ]; then
    echo "Removing file $file"
    rm "$file"
  fi
done

for dir in `find sounds -depth -type d`; do
  if [ -z "$( ls -A $dir )" ]; then
    echo "Removing empty dir $dir"
    rmdir "$dir"
  fi
done
