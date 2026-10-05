//! `cargo run -p pdf-engine --example optimize -- in.pdf out.pdf [lossless|balanced|smallest]`
use pdf_engine::optimize::{OptimizeOptions, optimize_bytes};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: optimize in.pdf out.pdf [lossless|balanced|smallest]");
        std::process::exit(2);
    }
    let opts = match a.get(3).map(String::as_str) {
        Some("lossless") => OptimizeOptions::lossless(),
        Some("smallest") => OptimizeOptions::smallest(),
        _ => OptimizeOptions::balanced(),
    };
    let input = match std::fs::read(&a[1]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {}: {e}", a[1]);
            std::process::exit(1);
        }
    };
    match optimize_bytes(&input, &opts) {
        Ok((out, rep)) => {
            if let Err(e) = std::fs::write(&a[2], &out) {
                eprintln!("cannot write {}: {e}", a[2]);
                std::process::exit(1);
            }
            println!("{rep:#?}");
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
