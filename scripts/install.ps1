# Harvest Coding Agent Installer for Windows

param(
    [switch]$Source,
    [switch]$Binary,
    [string]$Ref,
    [switch]$WithLaya,
    [switch]$Laya
)

$ErrorActionPreference = "Stop"

$Repo = if ($env:HARVEST_REPO) { $env:HARVEST_REPO } else { "Sanidhya14321/Harvest-Agent" }
$Package = "@harvest/pi-coding-agent"
$InstallDir = if ($env:PI_INSTALL_DIR) { $env:PI_INSTALL_DIR } else { "$env:LOCALAPPDATA\omp" }
$NativeArchitecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($NativeArchitecture -notin @("x64", "arm64")) {
    throw "Unsupported Windows architecture: $NativeArchitecture"
}
$BinaryName = "omp-windows-$NativeArchitecture.exe"
$MinimumBunVersion = "1.3.14"

function Test-BunInstalled {
    try {
        $null = Get-Command bun -ErrorAction Stop
        return $true
    } catch {
        return $false
    }
}

function Get-BunVersion {
    try {
        $versionText = (bun --version 2>$null)
        if (-not $versionText) {
            return $null
        }

        $clean = $versionText.Trim().Split("-")[0]
        return [version]$clean
    } catch {
        return $null
    }
}

function Test-BunVersion {
    param([string]$MinimumVersion)

    $currentVersion = Get-BunVersion
    if (-not $currentVersion) {
        return $false
    }

    return $currentVersion -ge [version]$MinimumVersion
}

function Assert-BunVersion {
    param([string]$MinimumVersion)

    if (-not (Test-BunVersion $MinimumVersion)) {
        $current = Get-BunVersion
        $currentText = if ($current) { $current.ToString() } else { "unknown" }
        throw "Bun $MinimumVersion or newer is required. Current version: $currentText. Upgrade Bun at https://bun.sh/docs/installation"
    }
}

function Test-GitInstalled {
    try {
        $null = Get-Command git -ErrorAction Stop
        return $true
    } catch {
        return $false
    }
}

function Test-GitLfsInstalled {
    try {
        $null = Get-Command git-lfs -ErrorAction Stop
        return $true
    } catch {
        return $false
    }
}

function Find-BashShell {
    # Check Git Bash first (most common on Windows)
    $gitBash = "C:\Program Files\Git\bin\bash.exe"
    if (Test-Path $gitBash) {
        return $gitBash
    }

    # Check bash.exe on PATH (Cygwin, MSYS2, WSL)
    try {
        $bashCmd = Get-Command bash.exe -ErrorAction Stop
        return $bashCmd.Source
    } catch {
        return $null
    }
}

function Configure-BashShell {
    try {
        $settingsDir = Join-Path $env:USERPROFILE ".omp\agent"
        $settingsFile = Join-Path $settingsDir "settings.json"

        # Check if settings.json already has a shellPath configured
        if (Test-Path $settingsFile) {
            try {
                $existingSettings = Get-Content $settingsFile -Raw | ConvertFrom-Json
                if ($existingSettings.shellPath) {
                    Write-Host "Bash shell already configured: $($existingSettings.shellPath)" -ForegroundColor Cyan
                    return
                }
            } catch {
                # Invalid JSON, we'll overwrite it
            }
        }

        $bashPath = Find-BashShell

        if ($bashPath) {
            Write-Host "Found bash shell: $bashPath" -ForegroundColor Cyan

            # Create settings directory if needed
            if (-not (Test-Path $settingsDir)) {
                New-Item -ItemType Directory -Force -Path $settingsDir | Out-Null
            }

            # Read existing settings or create new. ConvertFrom-Json -AsHashtable
            # requires PowerShell 6+; build the hashtable manually so Windows
            # PowerShell 5.1 merges instead of clobbering existing settings.
            $settings = @{}
            if (Test-Path $settingsFile) {
                try {
                    $parsed = Get-Content $settingsFile -Raw | ConvertFrom-Json
                    foreach ($prop in $parsed.PSObject.Properties) {
                        $settings[$prop.Name] = $prop.Value
                    }
                } catch {
                    $settings = @{}
                }
            }

            # Set shellPath
            $settings["shellPath"] = $bashPath

            # Write settings
            $settings | ConvertTo-Json -Depth 10 | Set-Content $settingsFile -Encoding UTF8
            Write-Host "[OK] Configured shell path in $settingsFile" -ForegroundColor Green
        } else {
            Write-Host ""
            Write-Host "No bash shell found - OMP will use its built-in shell." -ForegroundColor Cyan
            Write-Host "  For shell snapshots and interactive terminals, install Git for Windows:" -ForegroundColor Cyan
            Write-Host "    https://git-scm.com/download/win" -ForegroundColor Cyan
            Write-Host "  Or set a custom path in:" -ForegroundColor Cyan
            Write-Host "    $settingsFile" -ForegroundColor Cyan
            Write-Host '    { "shellPath": "C:\\path\\to\\bash.exe" }' -ForegroundColor Cyan
        }
    } catch {
        Write-Host "[WARN] Could not configure bash shell: $_" -ForegroundColor Yellow
    }
}

function Install-Bun {
    Write-Host "Installing bun..."
    irm bun.sh/install.ps1 | iex
    # Refresh PATH
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "User") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "Machine")
    Assert-BunVersion $MinimumBunVersion
}

function Install-ViaBun {
    Write-Host "Installing via bun..."
    if ($Ref) {
        if (-not (Test-GitInstalled)) {
            throw "git is required for -Ref when installing from source"
        }

        $tmpRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("omp-install-" + [System.Guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Force -Path $tmpRoot | Out-Null

        try {
            $repoUrl = "https://github.com/$Repo.git"
            $cloneOk = $false
            try {
                git clone --depth 1 --branch $Ref $repoUrl $tmpRoot | Out-Null
                $cloneOk = $true
            } catch {
                $cloneOk = $false
            }

            if (-not $cloneOk) {
                git clone $repoUrl $tmpRoot | Out-Null
                Push-Location $tmpRoot
                try {
                    git checkout $Ref | Out-Null
                } finally {
                    Pop-Location
                }
            }

            # Pull LFS files
            if (Test-GitLfsInstalled) {
                Push-Location $tmpRoot
                try {
                    git lfs pull | Out-Null
                } finally {
                    Pop-Location
                }
            }

            $packagePath = Join-Path $tmpRoot "packages\coding-agent"
            if (-not (Test-Path $packagePath)) {
                throw "Expected package at $packagePath"
            }

            bun install -g $packagePath
            if ($LASTEXITCODE -ne 0) {
                throw "Failed to install from $packagePath via bun"
            }
        } finally {
            Remove-Item -Recurse -Force $tmpRoot -ErrorAction SilentlyContinue
        }
    } else {
        bun install -g $Package
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to install $Package via bun"
        }
    }

    Write-Host ""
    Write-Host "[OK] Installed omp via bun" -ForegroundColor Green

    Configure-BashShell

    Write-Host "Run 'omp' to get started!"
}

function Install-Binary {
    if ($Ref) {
        Write-Host "Fetching release $Ref..."
        try {
            $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/tags/$Ref" -TimeoutSec 60
        } catch {
            throw "Release tag not found: $Ref`nFor branch/commit installs, use -Source with -Ref."
        }
    } else {
        Write-Host "Fetching latest release..."
        $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -TimeoutSec 60
    }

    $Latest = $Release.tag_name
    if (-not $Latest) {
        throw "Failed to fetch release tag"
    }
    Write-Host "Using version: $Latest"

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

    # Verify and launch the download before replacing an existing installation.
    $BinaryUrl = "https://github.com/$Repo/releases/download/$Latest/$BinaryName"
    Write-Host "Downloading $BinaryName..."
    $OutPath = Join-Path $InstallDir "omp.exe"
    $TempDir = Join-Path $InstallDir (".omp-download-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
    try {
        $Downloaded = Join-Path $TempDir $BinaryName
        $Checksums = Join-Path $TempDir "SHA256SUMS.txt"
        Invoke-WebRequest -Uri $BinaryUrl -OutFile $Downloaded -TimeoutSec 900 -UseBasicParsing
        Invoke-WebRequest -Uri "https://github.com/$Repo/releases/download/$Latest/SHA256SUMS.txt" -OutFile $Checksums -TimeoutSec 60 -UseBasicParsing
        $ChecksumLines = @(Get-Content $Checksums | Where-Object { $_ -match "^([a-fA-F0-9]{64})  $([regex]::Escape($BinaryName))$" })
        if ($ChecksumLines.Count -ne 1) {
            throw "Missing or invalid checksum for $BinaryName"
        }
        $ExpectedHash = $ChecksumLines[0].Substring(0, 64)
        $ActualHash = (Get-FileHash -Algorithm SHA256 -Path $Downloaded).Hash
        if ($ExpectedHash -ine $ActualHash) {
            throw "Checksum mismatch for $BinaryName; existing installation was not changed"
        }
        $VersionOutput = & $Downloaded --version 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "Downloaded omp cannot start: $VersionOutput"
        }
        Move-Item -Force -Path $Downloaded -Destination $OutPath
    } finally {
        Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
    }

    Write-Host ""
    Write-Host "[OK] Installed omp to $OutPath" -ForegroundColor Green

    # Add to PATH if not already there
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $needsRestart = $UserPath -notlike "*$InstallDir*"
    if ($needsRestart) {
        Write-Host "Adding $InstallDir to PATH..."
        [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    }

    Configure-BashShell

    $env:Path = "$InstallDir;$env:Path"
    Write-Host "Run 'omp' to get started! (New terminals will also find it.)"
}

# Main logic
if ($Ref -and -not $Source -and -not $Binary) {
    if ($Ref -match '^v[0-9]+\.[0-9]+\.[0-9]+$') {
        $Binary = $true
    } else {
        $Source = $true
    }
}

if ($Source) {
    if (-not (Test-BunInstalled)) {
        Install-Bun
    }
    Assert-BunVersion $MinimumBunVersion
    Install-ViaBun
} elseif ($Binary) {
    Install-Binary
} else {
    Install-Binary
}

if ($WithLaya -or $Laya) {
    Write-Host ""
    Write-Host "Configuring local Laya decision sidecar..." -ForegroundColor Cyan
    try {
        if (Test-Path (Join-Path $InstallDir "omp.exe")) {
            & (Join-Path $InstallDir "omp.exe") setup laya
        } else {
            bun x @harvest/pi-coding-agent setup laya
        }
        if ($LASTEXITCODE -eq 0) {
            Write-Host "[OK] Harvest + Laya setup complete!" -ForegroundColor Green
        } else {
            Write-Warning "Laya decision sidecar setup failed, but core Harvest is fully installed and operational. Run 'omp setup laya' to retry."
        }
    } catch {
        Write-Warning "Laya decision sidecar setup encountered an error: $_. Core Harvest is fully installed and operational. Run 'omp setup laya' to retry."
    }
}

