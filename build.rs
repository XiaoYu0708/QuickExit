// GNU 工具鏈（rustup 內建的精簡 mingw）沒有附 libshlwapi.a，但 eframe 依賴的 webbrowser 會連結它。
// 需要的函式其實已經在 windows-targets 提供的 libwindows.0.52.0.a 裡，
// 所以這裡只放一個空的 libshlwapi.a 讓連結器找得到檔案即可。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("link");
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
}
