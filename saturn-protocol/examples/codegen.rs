//! `generated/`의 JSON Schema와 TypeScript 타입을 다시 쓴다.
//!
//! 실행: `cargo run -p saturn-protocol --example codegen`

use std::path::Path;

use saturn_protocol::codegen;

fn main() -> std::io::Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("generated");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(codegen::SCHEMA_FILE), codegen::json_schema())?;
    std::fs::write(dir.join(codegen::TYPESCRIPT_FILE), codegen::typescript())?;
    println!("wrote {}", dir.display());
    Ok(())
}
