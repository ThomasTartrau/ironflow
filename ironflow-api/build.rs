fn main() {
    if std::env::var("IRONFLOW_DASHBOARD_DIR").is_err() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

        // When published to crates.io, the CI copies the built dashboard
        // into `dashboard/` inside the crate package. In the monorepo,
        // it lives at `../ironflow-dashboard/dist`.
        let embedded_path = format!("{manifest_dir}/dashboard");
        let monorepo_path = format!("{manifest_dir}/../ironflow-dashboard/dist");

        let path = if std::path::Path::new(&embedded_path)
            .join("index.html")
            .exists()
        {
            embedded_path
        } else {
            monorepo_path
        };

        println!("cargo:rustc-env=IRONFLOW_DASHBOARD_DIR={path}");
    }
}
