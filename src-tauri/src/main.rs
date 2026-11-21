fn main() {
    if let Err(error) = syncup_lib::run() {
        eprintln!("SyncUp failed to start: {error}");
        std::process::exit(1);
    }
}
