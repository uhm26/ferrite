fn main() {
    if let Err(e) = screenshot::app::run(std::env::args().skip(1)) {
        eprintln!("screenshot: {e}");
        std::process::exit(1);
    }
}
