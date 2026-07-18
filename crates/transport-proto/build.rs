fn main() -> std::io::Result<()> {
    // Requires `protoc` on PATH (or $PROTOC). CI installs it; the perf runner
    // lists it as a prerequisite.
    prost_build::compile_protos(&["proto/meridian.proto"], &["proto/"])
}
