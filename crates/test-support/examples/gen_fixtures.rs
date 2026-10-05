//! Write the programmatic fixtures to a directory: `cargo run -p test-support --example
//! gen_fixtures -- <dir>`. Used for manual GUI checks; tests call the builders directly.
use std::path::PathBuf;
use test_support::fixtures::*;

fn main() -> std::io::Result<()> {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    std::fs::create_dir_all(&dir)?;
    let all: [(&str, Vec<u8>); 8] = [
        ("helvetica_lines.pdf", helvetica_lines()),
        ("shared_content_stream.pdf", shared_content_stream()),
        ("image_page.pdf", image_page()),
        ("odd_geometry.pdf", odd_geometry()),
        ("mixed_sizes.pdf", mixed_sizes()),
        ("existing_annotations.pdf", existing_annotations()),
        ("acroform_basic.pdf", acroform_basic()),
        ("form_rich.pdf", form_rich()),
    ];
    for (name, bytes) in all {
        std::fs::write(dir.join(name), bytes)?;
    }
    Ok(())
}
