//! Prints the installed font families the scanner finds (for checking on a machine).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]
fn main() {
    let n = pdf_engine::sysfonts::scan_and_register();
    println!("{n} families");
    for (id, name) in pdf_engine::sysfonts::list() {
        let faces = pdf_engine::sysfonts::with_family(id, |f| {
            f.faces
                .iter()
                .map(|x| {
                    format!(
                        "{}{}",
                        if x.bold { "B" } else { "" },
                        if x.italic { "I" } else { "" }
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap();
        println!("{name}: [{faces}]");
    }
}
