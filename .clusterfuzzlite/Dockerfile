# ClusterFuzzLite build image. The base image carries nightly Rust, cargo-fuzz
# and the sanitizer runtimes, which is everything `fuzz/` needs.
FROM gcr.io/oss-fuzz-base/base-builder-rust
COPY . $SRC/aura-lang
WORKDIR $SRC/aura-lang
COPY .clusterfuzzlite/build.sh $SRC/
