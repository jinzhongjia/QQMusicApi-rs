use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=windows/qqm.manifest");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    // 只为 qqm 可执行文件声明身份；嵌入本库的其他程序拥有各自的应用身份。
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("windows/qqm.manifest");
    println!("cargo:rustc-link-arg-bin=qqm=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bin=qqm=/MANIFESTINPUT:{}", manifest.display());
}
