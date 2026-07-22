fn main() {
    println!("cargo:rerun-if-changed=../../packaging/assets/archductor.ico");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("../../packaging/assets/archductor.ico")
        .compile()
        .expect("embed Archductor application icon");
}
