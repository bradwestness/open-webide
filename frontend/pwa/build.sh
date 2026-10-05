#!/bin/sh
set -eu
dist=${TRUNK_STAGING_DIR:?Trunk staging directory is required}
for file in manifest.webmanifest pwa.js offline.html icon-192.png icon-512.png; do
  cp "pwa/$file" "$dist/$file"
done
version=$(cksum "$dist/index.html" "$dist/pwa.js" "$dist/manifest.webmanifest" "$dist/offline.html" "$dist/icon-192.png" "$dist/icon-512.png" pwa/service-worker.js pwa/build.sh | cksum | cut -d ' ' -f 1)
{
  printf 'const BUILD_ID = "%s";\nconst SHELL_FILES = ["/", "/offline.html", "/manifest.webmanifest", "/icon-192.png", "/icon-512.png", "/pwa.js"' "$version"
  for file in "$dist"/*.wasm "$dist"/*.js "$dist"/*.css; do
    [ -f "$file" ] || continue
    name=${file##*/}
    [ "$name" = pwa.js ] && continue
    [ "$name" = service-worker.js ] && continue
    printf ',"/%s"' "$name"
  done
  if [ -d "$dist/snippets" ]; then
    find "$dist/snippets" -type f -name '*.js' | sort | while IFS= read -r file; do
      printf ',"/%s"' "${file#"$dist"/}"
    done
  fi
  printf '];\n'
  cat pwa/service-worker.js
} > "$dist/service-worker.js"
