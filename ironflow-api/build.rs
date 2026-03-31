fn main() {
    if std::env::var("IRONFLOW_DASHBOARD_DIR").is_err() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let default_path = format!("{manifest_dir}/../ironflow-dashboard/dist");
        println!("cargo:rustc-env=IRONFLOW_DASHBOARD_DIR={default_path}");
    }
}
