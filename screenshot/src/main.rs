fn main() {
    if let Err(e) = snap::app::run(std::env::args().skip(1)) {
        eprintln!("snap: {e}");
        std::process::exit(1);
    }
}
