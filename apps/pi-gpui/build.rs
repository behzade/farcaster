fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        // Ghostty's static fontconfig archive leaves these symbols for the final link.
        println!("cargo:rustc-link-arg=-lxml2");
    }
}
