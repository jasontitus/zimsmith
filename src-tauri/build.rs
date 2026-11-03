fn main() {
    println!("cargo:rerun-if-env-changed=ZIMSMITH_UPDATE_FEED_URL");
    tauri_build::build()
}
