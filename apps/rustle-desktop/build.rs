fn main() {
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
            return;
        }
        let source = std::path::PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory is missing"),
        )
        .join("../../assets/icons/icon.png");
        println!("cargo:rerun-if-changed={}", source.display());
        let icon_path = std::path::PathBuf::from(
            std::env::var_os("OUT_DIR").expect("Cargo output directory is missing"),
        )
        .join("icon.ico");
        let source = image::open(source).expect("failed to read Windows icon source PNG");
        let frames: Vec<_> = [16, 24, 32, 48, 64, 128, 256]
            .into_iter()
            .map(|size| {
                let pixels = source
                    .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                    .to_rgba8();
                image::codecs::ico::IcoFrame::as_png(
                    pixels.as_raw(),
                    size,
                    size,
                    image::ExtendedColorType::Rgba8,
                )
                .expect("failed to encode Windows icon frame")
            })
            .collect();
        image::codecs::ico::IcoEncoder::new(
            std::fs::File::create(&icon_path).expect("failed to create Windows icon"),
        )
        .encode_images(&frames)
        .expect("failed to write Windows icon");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(icon_path.to_str().expect("icon path is not valid UTF-8"));
        res.compile()
            .expect("failed to compile Windows resources, including the generated PE icon");
    }
}
