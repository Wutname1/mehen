const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
    </windowsSettings>
  </application>
</assembly>
"#;

fn main() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("icons/MehenIcon.ico");
    res.set("ProductName", "Mehen Setup");
    res.set("CompanyName", "Mehen");
    res.set("FileDescription", "Mehen Setup");
    res.set("LegalCopyright", "Copyright 2026 Mehen");
    res.set_manifest(MANIFEST);
    res.compile().expect("Failed to compile Windows resources");
}
