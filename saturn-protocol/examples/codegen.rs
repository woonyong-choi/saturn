//! `generated/`를 다시 쓴다.

#![expect(clippy::print_stdout, reason = "생성 결과 경로를 사용자에게 출력")]

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
