fn main() {
    // protox compiles the schema in pure Rust, so builds need no protoc install.
    let proto = "../proto/space_mmo.proto";
    println!("cargo:rerun-if-changed={proto}");
    let descriptors = protox::compile([proto], ["../proto"]).expect("schema should compile");
    prost_build::Config::new()
        .compile_fds(descriptors)
        .expect("code generation should succeed");
}
