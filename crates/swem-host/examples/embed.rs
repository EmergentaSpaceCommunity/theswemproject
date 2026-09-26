//! The Workbench, embedded: a page of code that assembles the product over a
//! data root and opens its door.
//!
//! `cargo run -p swem-host --example embed` prints the address; open it. The
//! root is `SWEM_EMBED_ROOT` when set, else a directory under the temp dir.

use swem_host::product::{DataRoot, Product};

#[tokio::main]
async fn main() -> Result<(), String> {
    let root = std::env::var_os("SWEM_EMBED_ROOT").map_or_else(
        || std::env::temp_dir().join("swem-embed"),
        std::path::PathBuf::from,
    );
    let served = Product::at(DataRoot::at(root))
        .assemble()?
        .serve(([127, 0, 0, 1], 0).into(), ([127, 0, 0, 1], 0).into(), None)
        .await?;
    println!("SWEM Workbench: {}", served.url);
    println!("App sandbox origin: http://{}", served.handle.sandbox_addr);
    std::future::pending::<()>().await;
    Ok(())
}
