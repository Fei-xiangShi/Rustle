fn main() {
    #[cfg(windows)]
    {
        let icon_path = std::path::Path::new("../../assets/icons/icon.ico");
        if icon_path.exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon_path.to_str().expect("icon path is not valid UTF-8"));
            res.compile()
                .expect("failed to compile Windows resources, including the PE icon");
        } else {
            println!("cargo:warning=icon.ico not found, skipping Windows PE icon embedding");
        }
    }
}
