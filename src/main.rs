fn main() {
    if let Err(error) = roswire::run() {
        error.print_to_stderr();
        std::process::exit(i32::from(error.exit_code()));
    }
}
