# KeySwitch 2.0

Ultra-fast Thai/English keyboard auto-switcher for Windows written in **Pure Rust + Native Win32**.

## Features
- **Instant Auto-Switch**: Automatically detects mistyped Thai/English words (e.g. `l;ylfu` -> `สวัสดี`, `กน ` -> `do `) and switches Windows input language seamlessly.
- **Minimal Footprint**: ~1.2 MB single binary, ~1.2 MB RAM, zero heavy GUI frameworks (`winapi` only).
- **Native Windows Tray & Settings**: System tray menu, Thai/English UI toggle, customizable shortcuts, single-instance guard, and native toast notifications.
- **Self-Contained Installer**: Rename the release binary to `install.exe` (or run with `--setup`) to launch the 3-step Setup Wizard.

## Build
```powershell
cargo test --release
cargo build --release
Copy-Item target\release\rightlang.exe install.exe
```
