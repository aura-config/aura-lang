#!/bin/bash -eu
# Build every cargo-fuzz target and hand it to ClusterFuzzLite.
#
# The list of targets is read from `cargo fuzz list`, never written out: the
# hand-written list in ci.yml named six targets while seven existed, and the one
# left out, fuzz_bridge, was the one that had found a real defect.

cd "$SRC/aura-lang"
cargo fuzz build -O

out_dir=fuzz/target/x86_64-unknown-linux-gnu/release

for target in $(cargo fuzz list); do
  cp "$out_dir/$target" "$OUT/"

  # Seed each target with its own committed corpus, plus the example manifests
  # and the foreign files they read. A target ignores what it cannot use.
  seeds=$(mktemp -d)
  if [ -d "fuzz/corpus/$target" ]; then
    cp -r "fuzz/corpus/$target/." "$seeds/"
  fi
  find examples -type f \( -name '*.aura' -o -name '*.yaml' -o -name '*.toml' -o -name '*.json' \) \
    -exec cp --backup=numbered {} "$seeds/" \;
  (cd "$seeds" && zip -qr "$OUT/${target}_seed_corpus.zip" .)
  rm -rf "$seeds"
done
