cask "niwa-garden" do
  version "1.0.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/quiinthieu/niwa/releases/download/v#{version}/Niwa-v#{version}-macos.dmg"
  name "Niwa"
  desc "Menu bar companion for the cozy idle terminal garden"
  homepage "https://github.com/quiinthieu/niwa"

  depends_on formula: "niwa"
  depends_on macos: :sonoma

  app "Niwa.app"

  zap trash: [
    "~/Library/Application Support/Niwa",
    "~/Library/Caches/io.github.quiinthieu.niwa",
    "~/Library/Preferences/io.github.quiinthieu.niwa.plist",
    "~/Library/Saved Application State/io.github.quiinthieu.niwa.savedState",
  ]

  caveats <<~EOS
    #{token} is signed ad-hoc when built without an Apple Developer ID certificate.
    If Gatekeeper blocks first launch, install with `--no-quarantine` or clear the
    quarantine attribute manually:
      brew install --cask --no-quarantine quiinthieu/tap/#{token}
      xattr -dr com.apple.quarantine "/Applications/Niwa.app"
  EOS
end
