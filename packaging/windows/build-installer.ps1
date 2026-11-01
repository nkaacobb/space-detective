# Builds the release exe and wraps it in an installer: dist\SpaceDetective-Setup-<version>.exe
# Needs Rust and Inno Setup (https://jrsoftware.org/isdl.php).
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\..\.."
# Cargo reads .cargo\config.toml and rust-toolchain.toml from the current folder, so build from the project root.
Push-Location $root
$oldFlags = $env:CARGO_ENCODED_RUSTFLAGS
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

    # Swap this PC's folder paths (which include the Windows user name) for neutral ones in the exe.
    # These flags replace the ones in .cargo\config.toml, so the static C runtime is listed again.
    $flags = @('-Ctarget-feature=+crt-static', "--remap-path-prefix=$env:USERPROFILE=~")
    if ($env:CARGO_HOME) { $flags += "--remap-path-prefix=$env:CARGO_HOME=cargo" }
    $flags += "--remap-path-prefix=$root=."
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]0x1f

    cargo build --release --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $iscc = @(
        "$env:ProgramFiles\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) {
        $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
        if (-not $iscc) { throw "Inno Setup not found. Install it from https://jrsoftware.org/isdl.php" }
    }

    & $iscc /Q "/DAppVersion=$version" "$PSScriptRoot\space-detective.iss"
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }

    Get-Item "dist\SpaceDetective-Setup-$version.exe"
}
finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $oldFlags
    Pop-Location
}
