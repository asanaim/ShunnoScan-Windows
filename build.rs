fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("ProductName", "Shunno Scan");
        res.set("FileDescription", "Shunno Scan — Offline Document Scanner & Multi-Script OCR");
        res.set("LegalCopyright", "MIT License — Shunno Suite");
        let _ = res.compile();
    }
}
