fn main() {
    let development = std::env::var("PROFILE").as_deref() == Ok("debug");
    if development {
        let mut config: serde_json::Value = std::env::var("TAURI_CONFIG")
            .map(|value| serde_json::from_str(&value).expect("TAURI_CONFIG 不是有效 JSON"))
            .unwrap_or_else(|_| serde_json::json!({}));
        config["bundle"]["externalBin"] = serde_json::json!([]);
        // 构建脚本此时未启动线程；仅阻止 Tauri 覆盖正在运行的旧 debug helper。
        unsafe { std::env::set_var("TAURI_CONFIG", config.to_string()) };
    }
    tauri_build::build();
    if !development {
        return;
    }
    let marker = std::path::Path::new("binaries/dev-helper.sha256");
    println!("cargo:rerun-if-changed={}", marker.display());
    if marker.exists() {
        let hash = std::fs::read_to_string(marker).expect("读取开发 helper 标记失败");
        let hash = hash.trim();
        assert!(
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "开发 helper 标记无效，请运行 npm run prepare:sidecar:dev"
        );
        let name = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            "cas-helper.exe"
        } else {
            "cas-helper"
        };
        let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/dev")
            .join(hash)
            .join(name);
        assert!(
            helper.is_file(),
            "开发 helper 不存在，请运行 npm run prepare:sidecar:dev"
        );
        println!("cargo:rustc-env=CAS_DEV_HELPER_PATH={}", helper.display());
    }
}
