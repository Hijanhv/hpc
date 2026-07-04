//! Compile the shared control-plane protocol into Rust types + a gRPC server
//! and client. The generated module is included from `src/proto.rs`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "../proto/hpc.proto";
    println!("cargo:rerun-if-changed={proto}");
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&[proto], &["../proto"])?;
    Ok(())
}
