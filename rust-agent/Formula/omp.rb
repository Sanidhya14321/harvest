class Omp < Formula
  desc "Rust coding agent for the terminal"
  homepage "https://github.com/Sanidhya14321/harvest"
  version "@VERSION@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Sanidhya14321/harvest/releases/download/v#{version}/omp-darwin-arm64",
          using: :nounzip,
          verified: "github.com/Sanidhya14321/harvest/"
      sha256 "@SHA256_MACOS_AARCH64@"
    end
    on_intel do
      url "https://github.com/Sanidhya14321/harvest/releases/download/v#{version}/omp-darwin-x64",
          using: :nounzip,
          verified: "github.com/Sanidhya14321/harvest/"
      sha256 "@SHA256_MACOS_X86_64@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Sanidhya14321/harvest/releases/download/v#{version}/omp-linux-musl-arm64",
          using: :nounzip,
          verified: "github.com/Sanidhya14321/harvest/"
      sha256 "@SHA256_LINUX_AARCH64@"
    end
    on_intel do
      url "https://github.com/Sanidhya14321/harvest/releases/download/v#{version}/omp-linux-musl-x64",
          using: :nounzip,
          verified: "github.com/Sanidhya14321/harvest/"
      sha256 "@SHA256_LINUX_X86_64@"
    end
  end

  livecheck do
    url :url
    strategy :github_latest
  end

  def install
    bin.install Dir["omp-*"].fetch(0) => "omp"
    chmod 0555, bin/"omp"
    bin.install_symlink bin/"omp" => "harvest"
    with_env(HOME: buildpath) do
      generate_completions_from_executable(bin/"omp", "completions", shells: [:bash, :zsh, :fish])
    end
    (man1/"omp.1").write Utils.safe_popen_read(bin/"omp", "man")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/omp --version")
    assert_match version.to_s, shell_output("#{bin}/harvest --version")
    assert_match "read", shell_output("#{bin}/omp tools")
  end
end
