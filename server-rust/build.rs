fn main() {
    let cargo_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let src_dir = std::path::Path::new(&cargo_dir).join("src");
    let core_file = src_dir.join("core-prompt.txt");
    let example_file = src_dir.join("core-prompt.example.txt");
    if !core_file.exists() && example_file.exists() {
        let _ = std::fs::copy(&example_file, &core_file);
    }
}
