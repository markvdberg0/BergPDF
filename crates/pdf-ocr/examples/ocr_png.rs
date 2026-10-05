//! `cargo run -p pdf-ocr --example ocr_png -- <models-dir> <image.png>` prints recognised words.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::panic
)]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let engine = pdf_ocr::Engine::load(std::path::Path::new(&args[1])).unwrap();
    let dec = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&args[2]).unwrap(),
    ));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    let ch = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        _ => panic!("unsupported"),
    };
    let rgb: Vec<u8> = buf[..info.buffer_size()]
        .chunks_exact(ch)
        .flat_map(|p| {
            if ch == 1 {
                [p[0], p[0], p[0]]
            } else {
                [p[0], p[1], p[2]]
            }
        })
        .collect();
    let t = std::time::Instant::now();
    let words = engine.recognize_rgb(&rgb, info.width, info.height).unwrap();
    println!("{} words in {:?}", words.len(), t.elapsed());
    let mut line = usize::MAX;
    for w in words {
        if w.line != line {
            println!();
            line = w.line;
        }
        print!("{} ", w.text);
    }
    println!();
}
