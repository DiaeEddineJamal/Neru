# Template: the Release workflow fills in the placeholders and uploads the result as a release asset.
# Copy that rendered neru.rb into Casks/ of github.com/DiaeEddineJamal/homebrew-neru.
cask "neru" do
  arch arm: "aarch64", intel: "x64"

  version "@@VERSION@@"
  sha256 arm:   "@@SHA256_DMG_AARCH64@@",
         intel: "@@SHA256_DMG_X64@@"

  url "https://github.com/DiaeEddineJamal/Neru/releases/download/v#{version}/Neru_#{version}_#{arch}.dmg"
  name "Neru"
  desc "Local-first coding agent that proposes changes you review"
  homepage "https://github.com/DiaeEddineJamal/Neru"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: ">= :big_sur"

  app "Neru.app"

  zap trash: [
    "~/Library/Application Support/dev.neru.desktop",
    "~/Library/Caches/dev.neru.desktop",
    "~/Library/Preferences/dev.neru.desktop.plist",
    "~/Library/Saved Application State/dev.neru.desktop.savedState",
    "~/Library/WebKit/dev.neru.desktop",
  ]

  caveats <<~EOS
    Neru is not code-signed yet. The first time, right-click Neru.app and choose Open.
  EOS
end
