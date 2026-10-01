fn main() {
    match ccword::run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("ccword: {err}");
            std::process::exit(1);
        }
    }
}
