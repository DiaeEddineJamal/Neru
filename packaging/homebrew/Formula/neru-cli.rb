# Template: the Release workflow fills in the placeholders and uploads the result as a release asset.
# Copy that rendered neru-cli.rb into Formula/ of github.com/DiaeEddineJamal/homebrew-neru.
class NeruCli < Formula
  desc "Neru in your terminal: a local-first coding agent"
  homepage "https://github.com/DiaeEddineJamal/Neru"
  version "@@VERSION@@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/DiaeEddineJamal/Neru/releases/download/v#{version}/neru-cli-aarch64-apple-darwin.tar.gz"
      sha256 "@@SHA256_CLI_AARCH64_APPLE_DARWIN@@"
    end
    on_intel do
      url "https://github.com/DiaeEddineJamal/Neru/releases/download/v#{version}/neru-cli-x86_64-apple-darwin.tar.gz"
      sha256 "@@SHA256_CLI_X86_64_APPLE_DARWIN@@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/DiaeEddineJamal/Neru/releases/download/v#{version}/neru-cli-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@@SHA256_CLI_AARCH64_UNKNOWN_LINUX_GNU@@"
    end
    on_intel do
      url "https://github.com/DiaeEddineJamal/Neru/releases/download/v#{version}/neru-cli-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@@SHA256_CLI_X86_64_UNKNOWN_LINUX_GNU@@"
    end
  end

  livecheck do
    url :stable
    strategy :github_latest
  end

  def install
    # The binary looks for skills/ beside itself, so keep both in libexec.
    libexec.install "neru", "skills"
    bin.write_exec_script libexec/"neru"
  end

  def caveats
    s = "Update with `brew upgrade neru-cli` rather than `neru update`."
    on_linux do
      s += "\nneru needs WebKitGTK 4.1: sudo apt install libwebkit2gtk-4.1-0"
    end
    s
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/neru --version")
  end
end
